//! Shockwave 3D scene renderer using WebGL2
//!
//! Renders W3dScene data to an offscreen FBO, producing a texture
//! that can be composited as a regular sprite in the 2D pipeline.

// NOTE: swapping this for `FxHashMap` is worth ~6.7% of frame time, but it
// changes HashMap ITERATION ORDER, which is observable anywhere the renderer
// takes "the first" camera/light/node out of a map. That is a plausible cause of
// a 3D view coming out from the wrong angle, so it is reverted until the
// order-dependent lookups are found and made explicit.
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
    console_warn, director::chunks::w3d::types::*, player::symbols::{builtin::BuiltInSymbol, symbol::Symbol}
};

const SCENE3D_LOG: bool = false;

/// Content signature of one resource's decoded meshes — everything that ends up
/// in its GPU buffers, plus the parameters that transform it on the way there.
///
/// Replaces a vertex+face COUNT signature. Counts can only ever prove a resource
/// is DIFFERENT, never that it is the same, so carry-over had to be gated on the
/// scene's global `mesh_content_version`; that made ONE new mesh (AreaZero
/// builds a fresh `newMesh` trail for every rocket fired) re-upload EVERY mesh
/// in the member. Measured: the same class of frame costs 0.2ms when the global
/// flag stays put and 133ms when it moves.
///
/// Hashing the real content lets each resource be judged on its own, so a new
/// resource costs one upload. Only paid on frames that rebuild at all.
fn mesh_content_signature(
    meshes: &[crate::director::chunks::w3d::types::ClodDecodedMesh],
    subdiv: Option<&(i32, f32)>,
    uv_gen_mode: Option<u8>,
) -> u64 {
    use std::hash::{Hash, Hasher};
    #[allow(deprecated)]
    let mut h = std::collections::hash_map::DefaultHasher::new();
    meshes.len().hash(&mut h);
    for m in meshes {
        m.positions.len().hash(&mut h);
        m.faces.len().hash(&mut h);
        for v in &m.positions { v[0].to_bits().hash(&mut h); v[1].to_bits().hash(&mut h); v[2].to_bits().hash(&mut h); }
        for v in &m.normals { v[0].to_bits().hash(&mut h); v[1].to_bits().hash(&mut h); v[2].to_bits().hash(&mut h); }
        for uvset in &m.tex_coords {
            uvset.len().hash(&mut h);
            for v in uvset { v[0].to_bits().hash(&mut h); v[1].to_bits().hash(&mut h); }
        }
        for f in &m.faces { f.hash(&mut h); }
        for c in &m.diffuse_colors { for x in c { x.to_bits().hash(&mut h); } }
        for c in &m.specular_colors { for x in c { x.to_bits().hash(&mut h); } }
        for b in &m.bone_indices { b.len().hash(&mut h); for x in b { x.hash(&mut h); } }
        for w in &m.bone_weights { w.len().hash(&mut h); for x in w { x.to_bits().hash(&mut h); } }
    }
    // `#sds` rewrites the geometry after decode, and the UV generator changes the
    // uploaded texcoords, so both must be part of the identity.
    if let Some((d, t)) = subdiv { d.hash(&mut h); t.to_bits().hash(&mut h); }
    uv_gen_mode.hash(&mut h);
    h.finish()
}

fn log(msg: &str) {
    if SCENE3D_LOG {
        debug!("[SCENE-3D] {}", msg);
    }
}

/// GPU state for a single Shockwave3D member
struct MemberGpuData {
    /// Mesh buffers keyed by resource name (matches ModelNode.model_resource_name)
    mesh_groups: HashMap<Symbol, Vec<Mesh3dBuffers>>,
    /// All meshes in upload order (fallback when no scene graph match)
    all_meshes: Vec<Mesh3dBuffers>,
    /// Texture images decoded and uploaded to GPU
    textures: HashMap<Symbol, WebGlTexture>,
    /// Texture dimensions (width, height) keyed by lowercase name
    texture_sizes: HashMap<Symbol, (u32, u32)>,
    /// Cube map textures (keyed by base name)
    cube_maps: HashMap<Symbol, WebGlTexture>,
    /// Cached inverse bind matrices per skeleton name
    inverse_bind_cache: HashMap<Symbol, Vec<[f32; 16]>>,
    /// Snapshot of scene content counts when GPU data was built
    /// GEOMETRY signature: (nodes, clod+raw meshes, shaders). Deliberately does
    /// NOT include the texture count — see `ensure_member_data`.
    scene_version: (usize, usize, usize),
    /// Per-resource mesh write counter each uploaded group was built from, so an
    /// unchanged resource can be carried over without comparing its contents.
    mesh_versions: HashMap<Symbol, u64>,
    /// The scene's bulk-geometry counter at upload time. While it holds still,
    /// `mesh_versions` accounts for every geometry change; once it moves, some
    /// change was unattributed and contents must be compared instead.
    mesh_bulk_version: u64,
    /// Scene's mesh_content_version at last upload
    mesh_content_version: u64,
    /// Signature of the `#sds` subdivision state (per-resource depth/tension/
    /// enabled) folded in at upload. Changing `sds.depth`/`tension` at runtime
    /// (the SubDivSurfaces slider) mutates only runtime_state, not the scene, so
    /// this forces a GPU rebuild when the subdivided geometry must change.
    sds_version: u64,
    /// Per-RESOURCE mesh signature at upload (vertex + face counts summed over the
    /// resource's meshes). `mesh_content_version` is a single global counter, so
    /// it says geometry changed SOMEWHERE but not where — Agent Free Ride bumps
    /// it by ~4 per rebuild against ~364 mesh resources, and the whole set was
    /// re-uploaded to service that. This gives per-resource granularity.
    mesh_signatures: HashMap<Symbol, u64>,
    /// Per-texture data length at upload time (for incremental re-upload detection)
    texture_versions: HashMap<Symbol, u64>,
    /// Scene's texture_content_version at last check
    texture_content_version: u64,
    /// Scene's texture_epoch at last build. A change means the per-texture write
    /// counters were REWOUND (resetWorld / revertToWorldDefaults) and nothing
    /// already on the GPU can be trusted.
    texture_epoch: u64,
    /// Texture names (lowercase) that contain alpha < 250 (need alpha blending)
    alpha_textures: std::collections::HashSet<Symbol>,
    /// Subset of `alpha_textures` whose alpha is a smooth ramp rather than a
    /// binary cutout mask — these must be drawn in the blended transparent pass,
    /// never alpha-tested. See `SOFT_ALPHA_FRACTION`.
    soft_alpha_textures: std::collections::HashSet<Symbol>,
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
    u_flat_shading: Option<WebGlUniformLocation>,
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
    u_light_spot_exp: Option<WebGlUniformLocation>,
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
    /// Lower-cased name of the layer bound as diffuse, so a caller can look it
    /// up in `alpha_textures` / `soft_alpha_textures` without re-walking layers.
    diffuse_name: String,
}

/// What `bind_material_for_mesh` resolved for one mesh, so the caller can decide
/// how that mesh composites without re-walking the shader/material tables.
struct MeshMatInfo {
    /// Material opacity (Director `shader.blend / 100`).
    opacity: f32,
    /// `effective_blend_func`: 1 = IFX_ADD (additive), anything else = normal.
    blend_func: u8,
    /// Lower-cased name of the texture bound as diffuse, "" when none.
    diffuse_name: String,
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
    u_outline_pixels: Option<WebGlUniformLocation>,
    u_far_only: Option<WebGlUniformLocation>,
    u_viewport: Option<WebGlUniformLocation>,
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
    /// Stage scale in force for this frame, set by the compositor before it
    /// renders a 3D sprite.
    ///
    /// Camera backdrops and overlays are positioned in the SPRITE's own
    /// coordinate space — `addBackdrop(tex, point(x, y), rotation)` takes movie
    /// pixels — but the viewport they are drawn into is the sprite's RENDER
    /// rect, which a scaled stage has already enlarged. Without this the
    /// backdrop keeps its authored size and origin inside a viewport several
    /// times larger: estate's sky stayed a small band across the top with bare
    /// clear colour under it. The 2D ortho below divides by this, so a
    /// movie-space quad fills the same fraction of the viewport at any scale.
    /// 1.0 for every unscaled movie.
    pub stage_scale: f32,
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
    motion_transforms: HashMap<Symbol, [f32; 16]>,
    /// Single-track keyframe (object) motions that REPLACE a node's local
    /// transform — Director keyframePlayer semantics: the keyframe stores the
    /// node's full local transform, so it overrides the base rather than
    /// multiplying onto it (motion_transforms). Used by the multi-player path.
    motion_replace_transforms: HashMap<Symbol, [f32; 16]>,
    pub active_camera: Option<Symbol>,
    /// Set when a non-looping motion reaches its end — caller should advance the queue
    pub motion_ended: bool,
    /// Track last motion name to detect changes (sync animation_time from runtime state)
    last_motion_name: Option<Symbol>,
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
            stage_scale: 1.0,
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
uniform mat4 u_bone_matrices[96];

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
    if (u_uv_proj_mode == 5 || u_skinning_enabled == -1) {
        // #wrapPlanar projects in model space, overlays already carry [0, 1] UVs; for
        // both the matrix is a tweak in that same space.
        v_texcoord = (u_tex_transform * vec4(base_uv, 0.0, 1.0)).xy;
    } else {
        // The texture matrix belongs in DIRECTOR's UV space, and the V flip comes after.
        //
        // A CLOD mesh stores `director_uv - 0.5` in BOTH components — measured by
        // dumping SweeTarts' exit-gate mesh through `meshDeform.textureCoordinateList`
        // in Director and against the same mesh here: Director (-0.4114, -0.4046)
        // where we store (-0.91138, -0.90457), and Director 0..1 on the other mesh
        // where we store ±0.503. So `base_uv` above is (director_u, 1 - director_v):
        // right for sampling, because GL's texture origin is the other corner, but the
        // WRONG space to transform in. Director rotates about UV (0, 0) — that gate's
        // swirl mesh is authored with its UVs centred on zero precisely so the rotation
        // lands in the middle of the quad — while a flipped V puts our origin at
        // director_v = 1, one whole texture away. The swirl orbited off its quad
        // instead of spinning in place.
        //
        // Identity is a no-op either way, so untransformed meshes are unaffected.
        vec2 director_uv = a_texcoord + 0.5;
        vec2 t = (u_tex_transform * vec4(director_uv, 0.0, 1.0)).xy;
        v_texcoord = vec2(t.x, 1.0 - t.y);
    }
    if (u_skinning_enabled == -1) {
        v_texcoord2 = a_texcoord2;  // overlay: pass through as-is
    } else if (u_texcoord2_direct > 0) {
        v_texcoord2 = vec2(a_texcoord2.x, 1.0 - a_texcoord2.y);  // meshDeform: flip V for Director→OpenGL
    } else {
        v_texcoord2 = vec2(a_texcoord2.x + 0.5, 0.5 - a_texcoord2.y);
    }
    v_view_dist = -view_pos.z;
    gl_Position = u_projection * view_pos;
    gl_PointSize = 2.0;  // for #point renderStyle (ignored when drawing tris/lines)
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
// `shader.flat` — one normal per FACE instead of the interpolated per-vertex
// normal (Director 11.5 Scripting Dictionary, #standard shader property).
uniform int u_flat_shading;
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
uniform float u_light_spot_exp[8];   // spot falloff exponent (0 = uniform/hard edge, SpotDecay off)
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
    // Flat shading: derive the face normal from the screen-space derivatives of
    // the world position, which is constant across a triangle. Preferred over a
    // `flat` varying qualifier because GL ES takes those from the PROVOKING
    // vertex — the LAST one, and not selectable in WebGL2 — whereas Director
    // documents flat shading as using the face's FIRST vertex. The geometric
    // normal sidesteps the disagreement entirely. Sign is irrelevant here: the
    // one-sided lighting below already flips the shading normal to face the
    // viewer.
    if (u_flat_shading > 0) {
        vec3 fdx = dFdx(v_position);
        vec3 fdy = dFdy(v_position);
        vec3 fn = cross(fdx, fdy);
        if (dot(fn, fn) > 1e-12) N = normalize(fn);
    }
    vec3 V = normalize(u_camera_pos - v_position);
    // Director/IFX lighting is ONE-SIDED (max(0,N·L)) — surfaces facing away from a
    // light fall into shadow, which is what carves the directional shading. Flip the
    // shading normal to face the viewer so genuinely double-sided geometry still
    // receives light without depending on winding (the projection Y-flip makes
    // gl_FrontFacing unreliable).
    vec3 Nl = (dot(N, V) < 0.0) ? -N : N;

    vec4 tex_sample = texture(u_diffuse_tex, v_texcoord);

    // Alpha-test cutout: opaque models whose texture carries alpha (e.g. frog01's
    // Flash bark/leaf textures) are drawn in the opaque pass; discard transparent
    // texels so they don't write depth and aren't sorted as translucent.
    // `u_has_texture > 0` guard: nothing unbinds texture unit 0 between draws, so
    // on an untextured model `tex_sample` is whatever the PREVIOUS model left
    // there. Only a model that actually has a diffuse texture may be alpha-tested
    // against it.
    if (u_has_texture > 0 && u_alpha_threshold > 0.0 && tex_sample.a < u_alpha_threshold) discard;

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
                        // IFX/OpenGL spot cone. When SpotDecay is ON the light carries a
                        // GL_SPOT_EXPONENT (u_light_spot_exp = log10(.04)/log10(cos(outer)),
                        // computed per light) so the beam is full at the centre and 0.04 at
                        // the outer edge. When SpotDecay is OFF the exponent is 0 → pow()==1
                        // → uniform intensity inside the cone with a hard cutoff, matching
                        // Director/IFX. A tiny smoothstep only anti-aliases the rim.
                        else atten *= pow(spot_cos, u_light_spot_exp[i]) * smoothstep(cone_cos, cone_cos + 0.02, spot_cos);
                    }
                }
                // Two-sided lighting: use abs(N·L) so back faces also receive light
                float diff = max(dot(Nl, L), 0.0);
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
        if (u_layer2_blend >= 5) {
            // Reflection / environment map (#reflection): sphere-mapped, then
            // composited by the layer's own blendFunctionList entry (Director 11.5
            // Scripting Dictionary, `blendFunctionList`). 5 = #blend (ratio set by
            // blendConstant), 6 = #add (clamped), 7 = #multiply, 8 = #replace.
            vec3 refl = texture(u_layer2_tex, sphere_map_uv(N, v_position)).rgb;
            if (u_layer2_blend == 6) {
                final_color = min(final_color + refl, vec3(1.0));
            } else if (u_layer2_blend == 7) {
                final_color *= refl;
            } else if (u_layer2_blend == 8) {
                final_color = refl;
            } else {
                final_color = mix(final_color, refl, u_layer2_intensity);
            }
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
                        // IFX/OpenGL spot cone. When SpotDecay is ON the light carries a
                        // GL_SPOT_EXPONENT (u_light_spot_exp = log10(.04)/log10(cos(outer)),
                        // computed per light) so the beam is full at the centre and 0.04 at
                        // the outer edge. When SpotDecay is OFF the exponent is 0 → pow()==1
                        // → uniform intensity inside the cone with a hard cutoff, matching
                        // Director/IFX. A tiny smoothstep only anti-aliases the rim.
                        else atten *= pow(spot_cos, u_light_spot_exp[i]) * smoothstep(cone_cos, cone_cos + 0.02, spot_cos);
                    }
                }

                // Two-sided lighting: use abs(N·L) so back faces also receive light
                float diff = max(dot(Nl, L), 0.0);
                if (u_shader_mode == 1 && u_toon_steps > 0.0) {
                    diff = floor(diff * u_toon_steps + 0.5) / u_toon_steps;
                }
                result += atten * u_light_color[i] * base_color * diff;

                if (u_shininess > 0.0 && diff > 0.0) {
                    vec3 H = normalize(L + V);
                    float spec = pow(max(dot(Nl, H), 0.0), u_shininess);
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
    float refl_alpha = 1.0;
    if (u_layer2_blend >= 5) {
        vec4 refl4 = texture(u_layer2_tex, sphere_map_uv(N, v_position));
        vec3 refl = refl4.rgb;
        if (u_layer2_blend == 6) {
            result = min(result + refl, vec3(1.0));
        } else if (u_layer2_blend == 7) {
            result *= refl;
            // A #multiply layer multiplies the surface's ALPHA as well as its colour:
            // where the layer's texel is transparent it contributes nothing and leaves
            // the surface transparent there. SweeTarts' level-3 mascot is a sphere with
            // NO diffuse texture whose only layer is a "transcrome" reflection map at
            // #multiply — in Director you see the level straight through it with just a
            // few bright chrome highlights, which is what makes it read as a bubble.
            // Ignoring the layer's alpha rendered it as a flat opaque cyan disc.
            // Deliberately limited to #multiply: #add and #blend composite colour at a
            // ratio and say nothing about coverage (Agent Free Ride's #add coins would
            // start punching holes in themselves).
            refl_alpha = refl4.a;
        } else if (u_layer2_blend == 8) {
            result = refl;
        } else {
            result = mix(result, refl, u_layer2_intensity);
        }
    }

    // Apply fog (shared with the textured path via apply_fog).
    result = apply_fog(result);

    // Untextured path only — the textured branch returned above with its own
    // `u_opacity * tex_sample.a`. There is no diffuse texture bound for THIS draw,
    // so `tex_sample` still holds a sample of the previously drawn model's texture
    // and its alpha must not modulate this surface. Folding it in made every
    // material-only translucent model as transparent as whatever happened to be
    // drawn before it — AreaZero's enemy health bar (shader.blend 90, a flat red
    // quad with no texture layers) came out as a washed-out pink smear that faded
    // in and out with the draw order instead of a solid red bar.
    float alpha = u_opacity * u_diffuse_color.a * refl_alpha;
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
            u_flat_shading: u("u_flat_shading"),
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
            u_light_spot_exp: u("u_light_spot_exp[0]"),
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

    // sizeRange is the sprite size in WORLD UNITS ("Particles are measured in
    // world units" — Director 11.5 Scripting Dictionary, "sizeRange"), so the quad
    // is `size` across. a_corner spans -1..1, hence the 0.5 half-extent.
    float size_factor = mix(u_size_start, u_size_end, v_age_ratio) * 0.5;

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
    // blendRange is the particle OPACITY as a PERCENTAGE: "must be greater than
    // or equal to 0.0 and less than or equal to 100.0. The default value for
    // this property is 100.0" (Director 11.5 Scripting Dictionary, "blendRange").
    // Treating it as a 0..1 alpha clamped every real percentage to fully opaque —
    // Agent Free Ride's landing dust (blendRange.start = 20) and snow spray (30)
    // came out as solid white blobs instead of faint puffs.
    float opacity = clamp(mix(u_blend_start, u_blend_end, v_age_ratio) / 100.0, 0.0, 1.0);

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
        gl.blend_func_separate(
            WebGl2RenderingContext::SRC_ALPHA,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
            // Alpha must accumulate COVERAGE, not be blended with the same
            // factors as colour. With blend_func the alpha channel gets
            // dst.a = src.a*src.a + dst.a*(1-src.a), so a translucent draw over
            // an opaque background LOWERS its alpha (0.3 over 1.0 -> 0.79).
            // For a directToStage 3D sprite that layer is then composited over
            // the 2D sprites, so every dust particle punched a hole through the
            // scene and revealed the 2D sprites behind it — Heatwave Racing's
            // loading text/bar (channels 51-53, live on the same frame as the
            // race) bled through wherever dust was drawn.
            WebGl2RenderingContext::ONE,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
        );
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
                if let Some(tex) = gpu_data.and_then(|d| d.textures.get(&Symbol::from_str(&ps.texture_name))) {
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
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> Result<(), JsValue> {
        // GEOMETRY signature only. `texture_images.len()` used to be part of
        // this, so simply ADDING a texture tore down and rebuilt every mesh and
        // re-decoded every image in the member — for a movie that bakes each
        // in-world string into a `newTexture` (AreaZero's `[M] Text Director`)
        // that fired the first time each caption appeared, which is exactly the
        // "sometimes it freezes" report: measured 96.4ms of a 100.6ms frame,
        // 65 textures re-uploaded, for one small text texture arriving.
        //
        // Textures are already handled without a rebuild: `texture_content_version`
        // below routes to `update_textures_incremental`, which uploads any
        // texture whose per-texture write counter moved — including one that was
        // not there before (absent counter != present counter). So a texture
        // appearing needs no geometry work at all.
        let current_version = (scene.nodes.len(), scene.clod_meshes.len() + scene.raw_meshes.len(), scene.shaders.len());
        // Map each model resource to the active `#sds` subdivision applied to a
        // model node using it: resource_name (lowercase) → (depth, tension).
        // Only enabled modifiers with depth ≥ 1 subdivide.
        let subdiv_map = Self::build_subdiv_map(scene, runtime_state);
        let sds_version = Self::sds_signature(&subdiv_map);
        if let Some(existing) = self.member_data.get(&key) {
            if existing.scene_version == current_version
                && existing.mesh_content_version == scene.mesh_content_version
                && existing.sds_version == sds_version
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
        // KEEP the outgoing entry: its GPU textures are reusable. A rebuild is
        // triggered by `scene_version`, which counts nodes/meshes/textures/
        // shaders — so cloning a single model or calling removeFromWorld
        // invalidated the whole entry and re-decoded EVERY JPEG in the scene.
        // A profile of a texture-heavy movie put `decode_and_upload_texture_impl`
        // at 53% of total time, with the scalar zune_jpeg decode alone at 38%.
        //
        // Dropping this map also leaked every GL texture it held: `WebGlTexture`
        // is a JS handle, so dropping the Rust value does NOT call
        // `gl.deleteTexture`. Whatever is not carried over below is deleted.
        let mut old_gpu = self.member_data.remove(&key);

        let mut mesh_groups: HashMap<Symbol, Vec<Mesh3dBuffers>> = HashMap::new();
        let mut mesh_signatures: HashMap<Symbol, u64> = HashMap::new();
        let mut mesh_versions: HashMap<Symbol, u64> = HashMap::new();
        let mut all_meshes = Vec::new();
        // Was ANY mesh rewritten since this GPU entry was built? When nothing
        // was, every resource still present is identical by definition and can
        // be carried over WITHOUT hashing it — which matters, because hashing
        // the whole scene's geometry costs 10-30ms on a big member. Measured:
        // making every rebuild hash unconditionally turned a 0.2ms rebuild into
        // a 30.7ms one. The hash is only worth paying when the scene says some
        // mesh actually moved and we need to find out which.
        let content_unchanged = old_gpu.as_ref().map_or(false, |o| {
            o.sds_version == sds_version && o.mesh_content_version == scene.mesh_content_version
        });
        // Every geometry change since this entry was built named its resource,
        // so `mesh_write_versions` is a complete account of what moved. `#sds`
        // rewrites geometry outside that bookkeeping, so it disqualifies too.
        let attributed = old_gpu.as_ref().map_or(false, |o| {
            o.sds_version == sds_version && o.mesh_bulk_version == scene.mesh_bulk_version
        });

        // Mesh carry-over is decided PER RESOURCE, in three tiers below:
        // the scene's own record of what changed when that is complete
        // (`attributed`), a whole-member shortcut when nothing changed at all
        // (`content_unchanged`), and a content hash otherwise.
        //
        // The original signature was vertex + face COUNTS, which can only prove
        // a resource is different and never that it is the same — geometry
        // animation moves vertices without changing counts — so it had to be
        // backed by the global `mesh_content_version`, and gating on
        // `sds_version` alone caused real regressions (Intel 3dText lost its
        // tunnelling, a Havok camera view came out from the wrong angle).

        // Collect resource names used by LIGHT nodes (to skip their geometry)
        let light_resources: std::collections::HashSet<Symbol> = scene.nodes.iter()
            .filter(|n| n.node_type == W3dNodeType::Light)
            .flat_map(|n| {
                let mut names = vec![];
                if !n.model_resource_name.is_empty() { names.push(n.model_resource_name); }
                if !n.resource_name.is_empty() && n.resource_name.as_str() != "." { names.push(n.resource_name); }
                names
            })
            .collect();

        // Upload CLOD meshes (skip light geometry)
        for (name, decoded_meshes) in &scene.clod_meshes {
            if light_resources.contains(name) {
                continue; // Skip light cone/sphere meshes
            }
            // Fast path 2: some mesh was rewritten, but every rewrite since this
            // entry was built named its resource, and THIS resource was not one
            // of them. Carry it over without hashing — this is what keeps the
            // cost of one new mesh proportional to that mesh rather than to the
            // whole member.
            if attributed && !content_unchanged {
                if let Some(old) = old_gpu.as_mut() {
                    if old.mesh_versions.get(name).copied().unwrap_or(0)
                        == scene.mesh_write_version(name)
                    {
                        if let Some(group) = old.mesh_groups.remove(name) {
                            let old_sig = old.mesh_signatures.get(name).copied().unwrap_or(0);
                            mesh_signatures.insert(*name, old_sig);
                            mesh_versions.insert(*name, scene.mesh_write_version(name));
                            mesh_groups.insert(name.clone(), group);
                            continue;
                        }
                    }
                }
            }
            // Fast path 1: no mesh was rewritten at all, so carry this resource
            // over as-is and keep its stored signature, which is still true of it.
            if content_unchanged {
                if let Some(old) = old_gpu.as_mut() {
                    if let Some(group) = old.mesh_groups.remove(name) {
                        let old_sig = old.mesh_signatures.get(name).copied().unwrap_or(0);
                        mesh_signatures.insert(*name, old_sig);
                        mesh_versions.insert(*name, scene.mesh_write_version(name));
                        mesh_groups.insert(name.clone(), group);
                        continue;
                    }
                }
            }
            let sig: u64 = mesh_content_signature(
                decoded_meshes,
                subdiv_map.get(&name.to_lowercase()),
                scene.model_resources.get(name).and_then(|r| r.uv_gen_mode),
            );
            if let Some(old) = old_gpu.as_mut() {
                if old.mesh_signatures.get(name) == Some(&sig) {
                    if let Some(group) = old.mesh_groups.remove(name) {
                        mesh_signatures.insert(*name, sig);
                        mesh_versions.insert(*name, scene.mesh_write_version(name));
                        mesh_groups.insert(name.clone(), group);
                        continue;
                    }
                }
            }
            let mut group = Vec::new();
            for mesh in decoded_meshes.iter() {
                if mesh.positions.is_empty() || mesh.faces.is_empty() {
                    continue;
                }
                // `#sds` modifier: if a model node using this resource has an
                // enabled subdivision modifier, replace the mesh with its
                // butterfly-subdivided form before upload. Bone/lightmap/vertex-
                // color attributes don't survive subdivision (SDS targets static
                // models), so they're dropped for the subdivided copy.
                let subdivided_mesh;
                let mesh: &crate::director::chunks::w3d::types::ClodDecodedMesh =
                    if let Some(&(depth, tension)) = subdiv_map.get(&name.to_lowercase()) {
                        let uv0 = mesh.tex_coords.first().cloned().unwrap_or_default();
                        let (p, n, u, f) = crate::director::chunks::w3d::subdivision::subdivide(
                            &mesh.positions, &mesh.normals, &uv0, &mesh.faces, depth as u32, tension,
                        );
                        subdivided_mesh = crate::director::chunks::w3d::types::ClodDecodedMesh {
                            name: mesh.name.clone(),
                            positions: p,
                            normals: n,
                            tex_coords: if u.is_empty() { Vec::new() } else { vec![u] },
                            faces: f,
                            diffuse_colors: Vec::new(),
                            specular_colors: Vec::new(),
                            bone_indices: Vec::new(),
                            bone_weights: Vec::new(),
                        };
                        &subdivided_mesh
                    } else {
                        mesh
                    };
                // Use decoded texcoords, or generate UVs based on resource UV generator mode
                let uv_gen_mode = scene.model_resources.get(name)
                    .and_then(|r| r.uv_gen_mode);
                let tc_data;
                let tc = if !mesh.tex_coords.is_empty() && !mesh.tex_coords[0].is_empty() {
                    let tcs = &mesh.tex_coords[0];
                    // Check if all texcoords are identical (needs UV generation)
                    let all_same = tcs.len() > 1 && tcs.iter().all(|t| (t[0] - tcs[0][0]).abs() < 0.001 && (t[1] - tcs[0][1]).abs() < 0.001);
                    if all_same && !mesh.positions.is_empty() {
                        tc_data = generate_uvs_by_mode(&mesh.positions, uv_gen_mode);
                        Some(tc_data.as_slice())
                    } else if tcs.len() < mesh.positions.len() {
                        // Same rule as the 2nd set below: a short attribute buffer
                        // kills the whole draw call in WebGL, so pad it out.
                        let mut v = tcs.clone();
                        let fill = *tcs.last().unwrap_or(&[0.0, 0.0]);
                        v.resize(mesh.positions.len(), fill);
                        tc_data = v;
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
                // Get 2nd UV set if available (for lightmap/shadow textures).
                //
                // It must cover EVERY vertex. A short attribute buffer makes WebGL
                // reject the draw outright ("attempt to access out of range vertices
                // in attribute N") and the mesh silently disappears — the whole draw
                // call, not just the missing vertices.
                //
                // Runtime-supplied UV sets routinely come up short: Burnin' Rubber's
                // garage feeds its lightmap channel from a baked table in a TEXT
                // member (`CopyTextureCoordinates` reading "GarageLightmap"), and that
                // table carries 4152 coordinates against the 4164 vertices our CLOD
                // decode produces — so the entire showroom vanished while the cars,
                // which have no second UV set, kept rendering. Pad instead: Director
                // simply leaves the uncovered vertices unlit by the lightmap.
                let tc2_padded;
                let tc2 = if mesh.tex_coords.len() >= 2 && !mesh.tex_coords[1].is_empty() {
                    let uv2 = &mesh.tex_coords[1];
                    if uv2.len() < mesh.positions.len() {
                        let mut v = uv2.clone();
                        let fill = *uv2.last().unwrap_or(&[0.0, 0.0]);
                        v.resize(mesh.positions.len(), fill);
                        tc2_padded = v;
                        Some(tc2_padded.as_slice())
                    } else {
                        Some(&uv2[..mesh.positions.len().min(uv2.len())])
                    }
                } else {
                    None
                };

                // Pack bone data (variable-length per-vertex → fixed vec4)
                let (bone_idx_packed, bone_wgt_packed);
                let (bi_opt, bw_opt) = if !mesh.bone_indices.is_empty() && !mesh.bone_weights.is_empty()
                    && mesh.bone_indices.len() == mesh.positions.len()
                {
                    let (idx_p, wgt_p) = pack_bone_influences_sorted(&mesh.bone_indices, &mesh.bone_weights);
                    bone_idx_packed = idx_p;
                    bone_wgt_packed = wgt_p;
                    // Diagnostic: log bone data stats for first mesh with bones
                    {
                        use std::sync::Mutex; use std::collections::HashSet;
                        static LOGGED_BD: Mutex<Option<HashSet<Symbol>>> = Mutex::new(None);
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
                // Which UV space is the file's 2nd set in? The CLOD decoder stores
                // coordinates PRE-CENTERED (-0.5..0.5) and the vertex shader undoes
                // that with (u+0.5, 0.5-v); a set already in [0,1] must bypass it or
                // it shifts to ~[0.5,1.5], where the forced CLAMP smears the atlas
                // edge across the whole surface.
                //
                // Both layouts occur, so read it off the DATA instead of assuming:
                // a negative coordinate can only come from the pre-centered space.
                // AreaZero's Hangar/HangarFloor/RoadBlock lightmap sets measure
                // u,v in -0.50..0.50 — pre-centered, exactly like their base set —
                // and forcing them direct sampled the atlas at negative u, which
                // clamped to a black edge. That is why the baked light contributed
                // nothing recognisable and had to be composited as ADD to look like
                // anything at all (docs/areazero/README.md §3.3, "scene too bright").
                if let Some(uv2) = tc2 {
                    let pre_centered = uv2.iter().any(|c| c[0] < -0.001 || c[1] < -0.001);
                    buffers.texcoord2_direct = !pre_centered;
                }
                group.push(buffers);
            }
            mesh_signatures.insert(*name, sig);
            mesh_versions.insert(*name, scene.mesh_write_version(name));
            mesh_groups.insert(name.clone(), group);
        }

        // Upload raw meshes to mesh_groups (keyed by name) so draw_model_node can find them
        for mesh in &scene.raw_meshes {
            if light_resources.contains(&mesh.name) {
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
        let mut texture_sizes: HashMap<Symbol, (u32, u32)> = HashMap::new();
        let mut alpha_textures = std::collections::HashSet::new();
        let mut soft_alpha_textures = std::collections::HashSet::new();
        for (tex_name, image_data) in &scene.texture_images {
            // A texture declared by `newTexture(name)` with no source carries no
            // pixels yet (Director's "Blank" texture). It exists as a name until a
            // `.member` / `.image` assignment fills it in — nothing to upload.
            if image_data.is_empty() { continue; }
            let lower = tex_name.as_lower_str();
            // The SkyLine* textures in this game are authored vertically inverted in
            // the W3D (the JPEGs are stored upside-down, while houses/buildings/icons
            // are stored right-side-up). The skyline mesh UVs use the same convention
            // as everything else, and the texture declarations carry no orientation
            // flag, so flip these on upload to render the horizon the right way up.
            // Same signature `update_textures_incremental` uses: the scene's
            // per-texture WRITE counter. Unchanged => hand the existing GPU
            // texture straight over and skip decode + upload entirely.
            // Byte length used to stand in for this and could only prove an
            // image DIFFERENT, never the same — a recolour that re-encodes to
            // the same size compared equal (Heatwave Daytona's selected car
            // rendered black off a stale texture), and a fixed-size HUD readout
            // never compared unequal at all (Rifleman's frozen clock).
            // Reuse is decided PER TEXTURE, by its own write counter. It used to be
            // gated on the scene-wide `texture_content_version` as well, which meant
            // touching ONE texture re-decoded every JPEG in the member: Agent Free
            // Ride's track build clones ~40 models, each copying a few textures in,
            // so the whole texture set was decoded over and over and the level took
            // ~50 s to load with `decode_and_upload_texture_impl` dominating the
            // profile. The scene-wide counter is still what triggers the incremental
            // pass in `ensure_member_data`; it just no longer vetoes carry-over.
            //
            // Safe because every write to `texture_images` now goes through
            // `put_texture_image`, which bumps this per-texture counter — including
            // `merge` (loadFile), which previously extended the map behind its back.
            let write_version = scene.texture_write_versions.get(tex_name).copied().unwrap_or(0);
            let epoch_same = old_gpu
                .as_ref()
                .map_or(false, |o| o.texture_epoch == scene.texture_epoch);
            if let Some(old) = old_gpu.as_mut().filter(|_| epoch_same) {
                if old.texture_versions.get(tex_name) == Some(&write_version) {
                    if let Some(tex) = old.textures.remove(tex_name) {
                        if let Some(sz) = old.texture_sizes.get(tex_name) {
                            texture_sizes.insert(*tex_name, *sz);
                        }
                        if old.alpha_textures.contains(tex_name) {
                            alpha_textures.insert(*tex_name);
                        }
                        if old.soft_alpha_textures.contains(tex_name) {
                            soft_alpha_textures.insert(*tex_name);
                        }
                        textures.insert(*tex_name, tex);
                        continue;
                    }
                }
            }
            let flip_v = lower.contains("skyline");
            if let Some((tex, w, h, has_alpha, soft_alpha)) = self.decode_and_upload_texture(context, image_data, flip_v, scene.texture_near_filtering(tex_name), scene.texture_quality.get(tex_name).map(|q| q.as_str()).as_deref()) {
                texture_sizes.insert(*tex_name, (w, h));
                if has_alpha {
                    alpha_textures.insert(tex_name.clone());
                }
                if soft_alpha {
                    soft_alpha_textures.insert(*tex_name);
                }
                textures.insert(*tex_name, tex);
            }
        }

        // Release whatever was NOT carried over. `WebGlTexture` is a JS handle,
        // so dropping the map frees no GPU memory — before this, every rebuild
        // leaked its entire texture set.
        if let Some(old) = old_gpu.take() {
            let gl = context.gl();
            for (_, tex) in old.textures.iter() {
                gl.delete_texture(Some(tex));
            }
            for (_, tex) in old.cube_maps.iter() {
                gl.delete_texture(Some(tex));
            }
            // Meshes not carried over. Each holds a VAO plus up to seven VBOs,
            // and nothing freed them before — Agent Free Ride rebuilds 200+
            // times a session against ~364 mesh resources, so it leaked the
            // scene's entire vertex data over and over.
            for (_, group) in old.mesh_groups.iter() {
                for m in group.iter() {
                    m.delete(gl);
                }
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
        for tex_name in scene.texture_images.keys() {
            texture_versions.insert(
                *tex_name,
                scene.texture_write_versions.get(tex_name).copied().unwrap_or(0),
            );
        }
        self.member_data.insert(key, MemberGpuData {
            mesh_groups, mesh_signatures, all_meshes, textures, texture_sizes, cube_maps, inverse_bind_cache,
            scene_version: current_version,
            mesh_versions,
            mesh_bulk_version: scene.mesh_bulk_version,
            mesh_content_version: scene.mesh_content_version,
            sds_version,
            texture_versions,
            texture_content_version: scene.texture_content_version,
            texture_epoch: scene.texture_epoch,
            alpha_textures,
            soft_alpha_textures,
        });
        Ok(())
    }

    /// Build the resource→(depth, tension) map for active `#sds` modifiers. A
    /// model node with an enabled SDS modifier (`sds.enabled`, `sds.depth ≥ 1`)
    /// subdivides the mesh(es) of the resource it references. Keyed by lowercase
    /// resource name to match the CLOD mesh group keys case-insensitively.
    fn build_subdiv_map(
        scene: &W3dScene,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> HashMap<String, (i32, f32)> {
        let mut map = HashMap::new();
        let rs = match runtime_state { Some(rs) => rs, None => return map };
        if rs.sds_state.is_empty() { return map; }
        for node in &scene.nodes {
            if node.node_type != W3dNodeType::Model { continue; }
            let sds = rs.sds_state.iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(&node.name.as_str()))
                .map(|(_, v)| v);
            let sds = match sds { Some(s) if s.enabled && s.depth >= 1 => s, _ => continue };
            let resource = if !node.model_resource_name.is_empty() {
                &node.model_resource_name
            } else {
                &node.resource_name
            };
            if resource.is_empty() { continue; }
            // Cap depth so a slider at max can't allocate an unbounded mesh
            // (Director itself clamps via a triangle/vertex budget; 4 levels =
            // ×256 faces is plenty for the readout range this movie uses).
            let depth = sds.depth.min(4);
            map.insert(resource.to_lowercase(), (depth, sds.tension));
        }
        map
    }

    /// Order-independent signature of the subdivision map, so a runtime
    /// `sds.depth`/`tension` change invalidates the cached GPU mesh.
    fn sds_signature(map: &HashMap<String, (i32, f32)>) -> u64 {
        let mut sig: u64 = 1469598103934665603; // FNV offset basis
        let mut entries: Vec<(&String, &(i32, f32))> = map.iter().collect();
        entries.sort_by(|a, b| a.0.cmp(b.0));
        for (name, (depth, tension)) in entries {
            for b in name.as_bytes() { sig = (sig ^ *b as u64).wrapping_mul(1099511628211); }
            sig = (sig ^ *depth as u64).wrapping_mul(1099511628211);
            sig = (sig ^ tension.to_bits() as u64).wrapping_mul(1099511628211);
        }
        sig
    }

    /// Decode JPEG/PNG image data and upload as WebGL texture (delegates to free function)
    fn decode_and_upload_texture(&self, context: &WebGL2Context, data: &[u8], flip_v: bool, near_filtering: bool, quality: Option<&str>) -> Option<(WebGlTexture, u32, u32, bool, bool)> {
        decode_and_upload_texture_impl(context, data, flip_v, near_filtering, quality)
    }

    /// Incrementally re-upload only changed/new textures to GPU
    fn update_textures_incremental(&mut self, context: &WebGL2Context, key: (i32, i32), scene: &W3dScene) {
        let gpu_data = match self.member_data.get_mut(&key) { Some(d) => d, None => return };

        for (tex_name, image_data) in &scene.texture_images {
            let lower = tex_name.as_lower_str();
            // The scene's per-texture write counter, NOT the byte length.
            // Length can only prove an image is different, never that it is the
            // same — and this function runs precisely when some texture DID
            // change. Rifleman's HUD clock regenerates a fixed 64x64 RGBA image
            // every second, so its length never moves and a length check froze
            // the on-screen clock until an unrelated scene change forced a full
            // rebuild (shooting something), which is what "the timer only
            // updates when I shoot" was.
            let write_version = scene.texture_write_versions.get(tex_name).copied().unwrap_or(0);
            let needs_upload = gpu_data.texture_versions.get(tex_name) != Some(&write_version);
            if needs_upload {
                let flip_v = lower.contains("skyline");
                if let Some((tex, w, h, has_alpha, soft_alpha)) = decode_and_upload_texture_impl(context, image_data, flip_v, scene.texture_near_filtering(tex_name), scene.texture_quality.get(tex_name).map(|q| q.as_str()).as_deref()) {
                    gpu_data.texture_sizes.insert(*tex_name, (w, h));
                    if has_alpha {
                        gpu_data.alpha_textures.insert(*tex_name);
                    } else {
                        gpu_data.alpha_textures.remove(&tex_name);
                    }
                    if soft_alpha {
                        gpu_data.soft_alpha_textures.insert(*tex_name);
                    } else {
                        gpu_data.soft_alpha_textures.remove(tex_name);
                    }
                    gpu_data.textures.insert(*tex_name, tex);
                    gpu_data.texture_versions.insert(*tex_name, write_version);
                }
            }
        }

        // Remove GPU textures no longer in the scene
        let scene_keys: std::collections::HashSet<Symbol> = scene.texture_images.keys().copied().collect();
        gpu_data.textures.retain(|k, _| scene_keys.contains(k));
        gpu_data.texture_sizes.retain(|k, _| scene_keys.contains(k));
        gpu_data.alpha_textures.retain(|k| scene_keys.contains(k));
        gpu_data.soft_alpha_textures.retain(|k| scene_keys.contains(k));
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
        self.ensure_member_data(context, member_key, scene, runtime_state)?;

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
            if let Some(mat) = scene.materials.iter().find(|m| !m.name.as_lower_str().contains("default")) {
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
                let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data, w3d_shader.shader_type);
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
                let mut drawn_resources: std::collections::HashSet<Symbol> = std::collections::HashSet::new();
                for node in &scene.nodes {
                    if node.node_type != W3dNodeType::Model { continue; }
                    if !node.model_resource_name.is_empty() {
                        drawn_resources.insert(node.model_resource_name);
                    } else if !node.resource_name.is_empty() {
                        drawn_resources.insert(Symbol::from_str(node.resource_name.as_str().trim()));
                    }
                }
                let mut min_x = f32::MAX;
                let mut max_x = f32::MIN;
                let mut min_y = f32::MAX;
                let mut max_y = f32::MIN;
                for (resource_name, meshes) in &scene.clod_meshes {
                    if !drawn_resources.is_empty()
                        && !drawn_resources.contains(resource_name)
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
        // Liveness marker: proves the running wasm carries the soft-alpha pass
        // classification. Printed once per page load.
        self.ensure_shader(context)?;
        self.ensure_fbo(context, width, height)?;
        self.ensure_member_data(context, member_key, scene, runtime_state)?;
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
                                // Same space test as the loader: a negative
                                // coordinate can only come from the pre-centered
                                // CLOD space. Passing this through is what keeps
                                // the re-upload from flipping the flag.
                                let uv2 = &mesh.tex_coords[1];
                                let direct = !uv2.iter().any(|c| c[0] < -0.001 || c[1] < -0.001);
                                mesh_buf.update_texcoord2(context.gl(), uv2, direct);
                                mesh_buf.meshdeform_uv_synced = true;
                                let resource_name = resource_name.as_str();
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

        // Apply fog from runtime state or default off. Fog belongs to the CAMERA
        // this pass renders through, not to the member: Burnin' Rubber's menu
        // fogs `CameraFire` (the tunnel) to white and draws the whole UI over it
        // through an unfogged orthographic `CameraMenu`, so a member-wide fog
        // whited out the menu as well. `camera_fog` falls back to the member's
        // fog_* fields for any camera the movie never fogged.
        if let Some(rs) = runtime_state {
            let fog = self.active_camera.as_ref()
                .and_then(|c| rs.camera_fog.get(c).copied())
                .unwrap_or(crate::player::cast_member::CameraFog {
                    enabled: rs.fog_enabled,
                    near: rs.fog_near,
                    far: rs.fog_far,
                    color: rs.fog_color,
                    mode: rs.fog_mode,
                });
            if fog.enabled {
                gl.uniform1i(shader.u_fog_enabled.as_ref(), 1);
                gl.uniform1f(shader.u_fog_near.as_ref(), fog.near);
                gl.uniform1f(shader.u_fog_far.as_ref(), fog.far);
                gl.uniform3f(shader.u_fog_color.as_ref(), fog.color.0, fog.color.1, fog.color.2);
                gl.uniform1i(shader.u_fog_mode.as_ref(), fog.mode as i32);
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
                // `camera.colorBuffer.clearValue` overrides the member's bgColor for
                // the camera this pass renders (Director 11.5 Scripting Dictionary,
                // "clearValue").
                // The camera THIS pass renders through — not merely the first
                // View node in the scene, which is a different camera as soon as
                // a sprite carries more than one.
                let cam_clear = self.active_camera.as_ref()
                    .and_then(|c| rs.camera_clear_values.get(c).copied())
                    .or_else(|| scene.nodes.iter()
                        .find(|n| n.node_type == W3dNodeType::View)
                        .and_then(|n| rs.camera_clear_values.get(&n.name).copied()));
                let (r, g, b) = cam_clear
                    .or(rs.background_color)
                    .unwrap_or((0, 0, 0));
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
                let cam_key = self.active_camera
                    .unwrap_or_else(|| Symbol::from_str("defaultview"));
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
            let detached_nodes: std::collections::HashSet<Symbol> = runtime_state
                .map(|rs| rs.detached_nodes.iter().map(|s| s.as_str().into()).collect())
                .unwrap_or_default();

            // Check if active camera has a rootNode filter
            let root_node_filter: Option<Symbol> = runtime_state.and_then(|rs| {
                self.active_camera.as_ref()
                    .and_then(|cam| rs.camera_root_nodes.get(&cam))
                    .cloned()
            });

            let model_nodes: Vec<&W3dNode> = scene.nodes.iter()
                .filter(|n| n.node_type == W3dNodeType::Model)
                .filter(|n| {
                    // Skip directly detached nodes
                    if detached_nodes.contains(&n.name) { return false; }

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
                        self.is_child_of(scene, n.name, *root)
                    } else {
                        // No rootNode: render world-visible models only.
                        // Skip models whose parent (or ancestor) is detached — they belong
                        // to a different camera's rootNode subtree (e.g., overlay HUD models
                        // parented to a detached "overlays" camera).
                        !self.has_detached_ancestor(scene, n.parent_name, &detached_nodes)
                    }
                })
                .collect();

            // One-time diagnostic logging per member
            if !self.logged_members.contains(&member_key) {
                self.logged_members.insert(member_key);
                let gpu_data = self.member_data.get(&member_key);
                let mesh_group_keys: Vec<Symbol> = gpu_data.map(|d| d.mesh_groups.keys().cloned().collect()).unwrap_or_default();
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
            // Per-model semantics apply as soon as any model OWNS a motion, not only
            // when two players happen to exist. Director 11.5 Scripting Dictionary,
            // `keyframePlayer (modifier)`: "the motions managed by the keyframePlayer
            // modifier animate the ENTIRE MODEL at once" — i.e. a motion drives the
            // model running it. The legacy branch instead applies a single-track motion
            // to whatever node the track is NAMED after, which is wrong whenever an
            // exporter hoists a node's animation onto a carrier: AreaZero's
            // "Dummy Animation Node Camera1" runs motion "Camera1-Key" whose one track
            // is named "Camera1" (the camera parented UNDER that dummy), so the legacy
            // path moved the child camera and left the carrier — the node the game
            // actually mimics — standing still. Multi-track (skeletal) motions land in
            // motion_transforms by bone name in either branch, so they are unaffected.
            let multi_player = runtime_state
                .map(|rs| {
                    rs.bones_players.len() > 1
                        || rs.bones_players.values().any(|bp| bp.current_motion.is_some())
                })
                .unwrap_or(false);
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
                        // A PAUSED player still holds its pose — that is what pause
                        // means. The clock only advances while playing (see
                        // events::tick_w3d_animations), so applying the motion here
                        // unconditionally freezes the model on its current frame
                        // rather than snapping it back to the authored node
                        // transform. Bottle Rocket pauses its can and rocket at load
                        // and expects them to stand in the clip's frame 0 until the
                        // launch resumes them.
                        let motion_name = match &bp.current_motion { Some(m) => m, None => continue };
                        let motion = match scene.motions.iter().find(|m| m.name.eq_ignore_ascii_case(motion_name.as_str())) {
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
                                self.motion_replace_transforms.insert(Symbol::from_str(&model_name.to_ascii_lowercase()), m);
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

                let current_motion_name = runtime_state.and_then(|rs| rs.current_motion);

                // Detect motion change — sync animation_time from runtime state
                let motion_changed = current_motion_name != self.last_motion_name;
                if motion_changed {
                    self.last_motion_name = current_motion_name;
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
                    scene.motions.iter().find(|m| m.name == name)
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
                // Fallback for a member with no model-node scene graph at all: draw
                // every mesh at identity so the geometry is at least visible.
                //
                // It must NOT fire when the scene HAS model nodes and the filtering
                // above merely excluded them — that emptiness is the intended result,
                // and dumping all meshes at identity paints flat untextured geometry
                // over whatever earlier camera passes drew. Director's standard
                // overlay-camera idiom hits this every frame: Agent Free Ride 2 does
                //   ingameCam = member.newCamera("ingame_cam")
                //   ingameCam.rootNode = member.newGroup("void_group")
                //   sprite.addCamera(ingameCam)
                // so the second pass legitimately renders no models (its rootNode
                // subtree is empty, it only carries overlays) — and the fallback was
                // wiping the whole 3D world to flat grey.
                let scene_has_model_nodes = scene.nodes.iter()
                    .any(|n| n.node_type == W3dNodeType::Model);
                if !scene_has_model_nodes {
                    self.draw_all_meshes_fallback(gl, shader, scene, &member_key);
                }
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
                    if n.name.as_lower_str().starts_with("sb_") && n.parent_name.as_lower_str().contains("skybox") { 0 } else { 1 }
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
                    // and shader.transparent, NOT on the W3D material's opacity field. A model
                    // is only actually see-through when the surface isn't an opaque textured
                    // solid — so an OPAQUE diffuse texture forces the opaque pass regardless of
                    // a low material opacity OR a `.transparent = 1` flag. This fixes two cases:
                    //   * finalDrive `chassis` (material opacity 0.2, opaque camo) — was 20%
                    //     see-through, showing the passengers through the car body.
                    //   * LEGO SuperSonic — its shaders inherit Director's default
                    //     `transparent = TRUE` (at blend=100 = opaque); tracking that flag
                    //     dumped every opaque-textured prop into the depth-off transparent pass,
                    //     so warehouse boxes/coils rendered as floating dark solids.
                    // Genuine translucents (galaxy glow's alpha texture, plain-colour water)
                    // have no opaque texture, so they stay in the transparent pass.
                    //
                    // A soft-alpha texture (a smooth alpha ramp, not a cutout mask) is
                    // translucent on its own account, whatever the material opacity says:
                    // the alpha-tested cutout pass would snap its ramp to on/off. AreaZero's
                    // MenuScanLines camera filter is exactly this — opacity 1.0, but 55% of
                    // its atlas texels carry intermediate alpha, so it was rendering as solid
                    // black bars instead of a vignette.
                    let has_soft_alpha = self.model_has_soft_alpha_texture(scene, model_node, &member_key, runtime_state);
                    // An additive surface is never opaque, whatever its texture looks
                    // like: it ADDS to the framebuffer, so it must be drawn blended and
                    // after the geometry it sits on top of. AreaZero's MenuCharacter FX
                    // (MuzzleFlash, BulletStreak*, Particle*, Smoke*, VisorAdd, …) bind
                    // an opaque texture, so the `!has_opaque_texture` test dropped all 28
                    // of them into the OPAQUE pass, where they wrote depth and drew
                    // nothing.
                    // An #add TEXTURE LAYER is a multi-texture blend INSIDE the
                    // material - that layer adds onto the layers beneath it. It does
                    // not mean the surface composites additively against the
                    // framebuffer. The additive idiom this test exists for is the
                    // one named above: shader.blend driven to 0 (opacity ~0), which
                    // would otherwise make the model invisible.
                    //
                    // age-of-speed's car chassis is opacity 1.0 with an additive
                    // detail layer; treating it as framebuffer-additive drew every
                    // car blown-out white. Require the idiom, not just the layer.
                    let is_additive = opacity < 0.999
                        && self.model_is_additive(scene, model_node, runtime_state);
                    let wants_transparent = Self::model_uses_transparent_shader(model_node, runtime_state)
                        || opacity < 0.999
                        || has_soft_alpha
                        || is_additive;
                    let is_transparent = wants_transparent
                        && (has_soft_alpha
                            || is_additive
                            || !self.model_has_opaque_texture(scene, model_node, &member_key, runtime_state));

                    // One-shot per (member, model): which of the three passes this
                    // model lands in, and the inputs that decided it. Deduped through
                    // a thread-local because `shader` holds a shared borrow of `self`
                    // for the whole of this function.
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
                    gl.blend_func_separate(
                        WebGl2RenderingContext::SRC_ALPHA,
                        WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
                        // Alpha must accumulate COVERAGE, not be blended with the same
                        // factors as colour. With blend_func the alpha channel gets
                        // dst.a = src.a*src.a + dst.a*(1-src.a), so a translucent draw over
                        // an opaque background LOWERS its alpha (0.3 over 1.0 -> 0.79).
                        // For a directToStage 3D sprite that layer is then composited over
                        // the 2D sprites, so every dust particle punched a hole through the
                        // scene and revealed the 2D sprites behind it — Heatwave Racing's
                        // loading text/bar (channels 51-53, live on the same frame as the
                        // race) bled through wherever dust was drawn.
                        WebGl2RenderingContext::ONE,
                        WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
                    );

                    for (model_node, _dist) in &transparent_nodes {
                        self.draw_model_node(gl, shader, scene, model_node, &member_key, runtime_state, &view_matrix, &projection_matrix, true);
                    }

                    gl.depth_mask(true);
                    gl.disable(WebGl2RenderingContext::BLEND);
                }
            }
        }

        // Render particles (after opaque geometry), alpha-blended.
        let _ = self.render_particles(context, &member_key, runtime_state, &view_matrix, &projection_matrix);

        // #inker outlines, LAST. The inverted hull needs the inked model's own depth to
        // clip it, and a translucent model wrote none (the transparent pass runs
        // depth_mask(false)), so the pass lays that depth down itself. Drawing it after the
        // particles keeps those extra depth writes from occluding anything: nothing but
        // post-processing follows.
        let _ = self.render_inker_outlines(context, scene, &member_key, &view_matrix, &projection_matrix, (width, height), runtime_state);

        // Re-activate main shader after the outline pass.
        if let Some(ref shader) = self.shader {
            gl.use_program(Some(&shader.program));
        }

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
        gl.blend_func_separate(
            WebGl2RenderingContext::SRC_ALPHA,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
            // Alpha must accumulate COVERAGE, not be blended with the same
            // factors as colour. With blend_func the alpha channel gets
            // dst.a = src.a*src.a + dst.a*(1-src.a), so a translucent draw over
            // an opaque background LOWERS its alpha (0.3 over 1.0 -> 0.79).
            // For a directToStage 3D sprite that layer is then composited over
            // the 2D sprites, so every dust particle punched a hole through the
            // scene and revealed the 2D sprites behind it — Heatwave Racing's
            // loading text/bar (channels 51-53, live on the same frame as the
            // race) bled through wherever dust was drawn.
            WebGl2RenderingContext::ONE,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
        );

        // Movie-space viewport — see `stage_scale` and draw_backdrops_inline.
        let s = if self.stage_scale > 0.0 { self.stage_scale } else { 1.0 };
        let w = width as f32 / s;
        let h = height as f32 / s;
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
            gl.uniform1i(shader.u_flat_shading.as_ref(), 0);

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
            // Rotation pivot. The dictionary says rotation is "about its
            // regPoint" and that regPoint defaults to point(0,0) — the texture's
            // UPPER-LEFT — but taken literally that spins an unrotated-regPoint
            // overlay around its own top corner, which no movie wants and
            // Director visibly does not do. Rifleman pins this down: its radar
            // view-cone is a 64x64 texture placed at `playerCentre - (32,32)`,
            // i.e. deliberately centred on the player dot, and then rotated to
            // the aim direction every frame. That only tracks the player if the
            // pivot is the quad's CENTRE; pivoting at the top-left swung the
            // cone around a point 32px up-left of the dot, so it changed
            // direction but never rotated about the player.
            // So: pivot at regPoint when a script actually set one (that is the
            // documented behaviour and what e.g. a scaled scope reticle asks
            // for), and at the quad centre when regPoint is merely sitting at
            // its default.
            //
            // translate = loc + anchor − R·pivot, where `pivot` is the point held
            // fixed by the rotation and `anchor` is where that point sits
            // relative to loc. With an explicit regPoint the two coincide
            // (anchor 0: loc IS the regPoint's screen position, the documented
            // meaning of loc); with the default they do not, because loc still
            // places the upper-left while the quad turns about its middle.
            let (ax, ay, px, py) = if overlay.reg_point_explicit {
                (0.0, 0.0, rx, ry)
            } else {
                let (cx, cy) = (sw * 0.5, sh * 0.5);
                (cx, cy, cx, cy)
            };
            let model: [f32; 16] = [
                cos_r * sw, sin_r * sw, 0.0, 0.0,
               -sin_r * sh, cos_r * sh, 0.0, 0.0,
                0.0,        0.0,        1.0, 0.0,
                x + ax - px * cos_r + py * sin_r,
                y + ay - px * sin_r - py * cos_r,
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
        gl.blend_func_separate(
            WebGl2RenderingContext::SRC_ALPHA,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
            // Alpha must accumulate COVERAGE, not be blended with the same
            // factors as colour. With blend_func the alpha channel gets
            // dst.a = src.a*src.a + dst.a*(1-src.a), so a translucent draw over
            // an opaque background LOWERS its alpha (0.3 over 1.0 -> 0.79).
            // For a directToStage 3D sprite that layer is then composited over
            // the 2D sprites, so every dust particle punched a hole through the
            // scene and revealed the 2D sprites behind it — Heatwave Racing's
            // loading text/bar (channels 51-53, live on the same frame as the
            // race) bled through wherever dust was drawn.
            WebGl2RenderingContext::ONE,
            WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
        );

        // Movie-space viewport: backdrops are authored in movie pixels, so the
        // ortho works in that space and the (already enlarged) GL viewport does
        // the scaling. See `stage_scale`.
        let s = if self.stage_scale > 0.0 { self.stage_scale } else { 1.0 };
        let w = width as f32 / s;
        let h = height as f32 / s;
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
            gl.uniform1i(shader.u_flat_shading.as_ref(), 0);

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
                node.model_resource_name
            } else {
                node.resource_name
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
            runtime_state.and_then(|rs| get_runtime_transform(rs, *&node.name));
        // Get this node's transform: motion (combined with base), runtime override, or parsed
        let node_transform = if allow_motion_override {
            if let Some(km) = (!self.motion_replace_transforms.is_empty())
                .then(|| self.motion_replace_transforms.get(&Symbol::from_str(&node.name.to_ascii_lowercase())))
                .flatten()
            {
                // Lookup is case-insensitive (bones_players keys are lowercased;
                // node names aren't).
                match runtime_override {
                    Some(rt) => mat4_multiply_col_major(&rt, km),
                    // Base FIRST, keyframe applied in the node's own frame — the same
                    // order as the runtime-override branch above, and the order the
                    // clips are authored in: an object keyframe starts at IDENTITY
                    // (pos 0, rot identity, scale 1) and is a delta from the node's
                    // authored rest pose. Agent Free Ride's paraglider canopy is the
                    // proof: `parachute`'s node is authored at ~1/100 scale and its
                    // clip ramps scale ~100x as the canopy inflates, with translation
                    // running to ~1e6 in that same 100x space. Composed the other way
                    // round (`km * base`) the clip's raw translation is NOT divided by
                    // the node's 1/100 scale, so the canopy was drawn ~1.4 MILLION
                    // units away — off screen, which is why the end-of-level shot had
                    // a boarder and no parachute.
                    None => mat4_multiply_col_major(&node.transform, km),
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
                    .and_then(|rs| get_runtime_transform(rs, parent_node.name))
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

    /// World-space bounding radius of a model whose resource is a RUNTIME PRIMITIVE
    /// (`newModelResource(name, #box/#sphere/#cylinder/#plane/…)`), whose dimensions
    /// the script set and we therefore know exactly. `None` for parsed CLOD meshes,
    /// where the vertices live on the GPU and there is no cheap extent to read.
    fn primitive_world_radius(res_info: Option<&ModelResourceInfo>, world: &[f32; 16]) -> Option<f32> {
        let info = res_info?;
        let kind = info.primitive_type.as_deref()?;
        // Half-extents in the resource's own space, per Director's primitive
        // dimension properties (width/length/height are FULL sizes; radius is not).
        let (w, l, h) = (
            0.5 * info.primitive_width.abs(),
            0.5 * info.primitive_length.abs(),
            0.5 * info.primitive_height.abs(),
        );
        let half = match kind {
            "box" => w.max(l).max(h),
            "plane" => w.max(l),
            "sphere" => info.primitive_radius.abs(),
            "cylinder" => info.primitive_radius.abs()
                .max(info.primitive_top_radius.abs())
                .max(h),
            // #particle and anything else has no meaningful authored extent.
            _ => return None,
        };
        // The largest axis scale in the world matrix — a sphere of this radius in
        // model space cannot exceed one of `half * scale` in world space.
        let axis = |c: usize| {
            (world[c * 4] * world[c * 4]
                + world[c * 4 + 1] * world[c * 4 + 1]
                + world[c * 4 + 2] * world[c * 4 + 2])
                .sqrt()
        };
        let scale = axis(0).max(axis(1)).max(axis(2));
        let r = half * scale;
        if r.is_finite() { Some(r) } else { None }
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
            model_node.model_resource_name
        } else {
            model_node.resource_name
        };
        let res_info = scene.model_resources.get(&resource);

        // Rasterwerks tags its skybox nodes `SB_*` under a `*SkyBox*` parent; other
        // movies just name a big camera-enclosing box "skybox" (unicraft clones one
        // from a cast member and scales it ×1000). Both need the same treatment:
        // render inside-out (no cull), camera-centered, past the normal far plane —
        // otherwise the box's inner faces are culled/clipped and the starfield
        // background is missing (only the foreground galaxy plane shows).
        let named_skybox = (model_node.name.starts_with("SB_") && model_node.parent_name.as_lower_str().contains("skybox"))
            || model_node.name.as_lower_str().contains("skybox");
        // …but the treatment is a RESCUE for geometry authored so far out that the
        // camera's real far plane clips it away, not something the name alone earns.
        // A movie is free to call an ordinary world object "skybox": SweeTarts 3D
        // builds `newModelResource("skybox", #cylinder, #back)` with radius 6000 —
        // well inside its camera's yon of 10000 — translates it to (0, -1000, 0),
        // parents the ground and ceiling caps to it and then, in the "mrseasick"
        // level, tilts the whole thing. Camera-centring that cylinder decoupled it
        // from its own caps, and the depth-mask-off pass stopped it occluding them,
        // so the 12000-unit ground plane's corners (which sit OUTSIDE the 6000 wall
        // and are meant to be hidden by it) drew straight over the jungle backdrop.
        // Only take over a model the camera's own far plane could not show.
        let is_skybox = named_skybox && {
            let world = self.accumulate_transform_with_state(scene, model_node, runtime_state);
            match Self::primitive_world_radius(res_info, &world) {
                // Deliberately compared against the model's own extent and NOT its
                // distance from the eye: a camera-position-dependent test would flip
                // the model between the two treatments as the camera roams, popping
                // the backdrop mid-frame.
                Some(radius) => {
                    let far = projection_matrix[14] / (projection_matrix[10] + 1.0);
                    !(far.is_finite() && far > 0.0 && radius < far)
                }
                // Parsed (non-primitive) geometry has no cheap extent here, so keep
                // the historical behaviour — that is the Rasterwerks/unicraft case.
                None => true,
            }
        };
        let mut vis_mode = 1u8; // default #front

        if let Some(gpu_data) = self.member_data.get(member_key) {
            let has_skeleton_data = self.setup_skinning_for_resource(
                gl, shader, scene, resource, model_node.name, gpu_data, runtime_state,
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
                    // No basis rebase for passive skinned content. A historical
                    // "Z-up -> render basis" column swap used to be applied here,
                    // but IFX is right-handed with NO axis remap anywhere else in
                    // this renderer (see the coordinate-conventions note: view =
                    // plain camera inverse, never negate axes), and Intel's own
                    // IFX sample DoNotPush.dcr is direct evidence against it: its
                    // 82-bone "Barry_delib" rig parses and skins correctly and the
                    // swap laid the character on its back, viewed from above.
                    // Measured: with the swap restored, `intel_do_not_push`'s
                    // start_game snapshot fails at 19.82% against its reference;
                    // without it, it passes. A per-draw probe also showed this
                    // branch is reached by only two things in the whole 3D suite
                    // — `Barry_delib` and AreaZero's `RobotGunShadow` — because
                    // every other skinned model is Lingo-driven and takes the
                    // override branch above. In particular NOTHING in Rifleman
                    // or Agent Free Ride reaches it, so this hunk cannot be
                    // (and was not) the cause of their rider/soldier regression;
                    // that was the clone tier below. Those two movies have no
                    // committed reference snapshots at all, so do not read a
                    // green run as rendering validation for them.
                    gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &world_matrix);
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
                if mode == 2 {
                    // #back — draw ONLY back faces, i.e. cull the front ones.
                    // This is not the same as #both: treating it as "disable
                    // culling" makes a #back model fully opaque from outside,
                    // which breaks the standard cartoon-outline trick — a
                    // slightly scaled-up black clone of a model drawn #back so
                    // only its far side shows as a rim. Heatwave Racing builds
                    // exactly that per car:
                    //   blackbodymodel = model("<car> body").cloneDeep(...)
                    //   blackbodymodel.shaderList[n] = <solid black shader>
                    //   blackbodymodel.visibility = #back
                    //   blackbodymodel.transform.scale = vector(bbsc,bbsc,bbsc)
                    // With culling off, that black shell covered the textured
                    // livery and every car rendered as a black silhouette.
                    // Note this renderer's global default is cull_face(FRONT)
                    // for #front, so #back is its opposite: BACK.
                    gl.enable(WebGl2RenderingContext::CULL_FACE);
                    gl.cull_face(WebGl2RenderingContext::BACK);
                } else if mode == 3 {
                    // #both — genuinely show both sides.
                    gl.disable(WebGl2RenderingContext::CULL_FACE);
                }
                mode
            };

            // See the per-mesh depth note in the loop: only a model whose transparent
            // classification came off a fully hidden mesh gets that treatment.
            let model_has_hidden_mesh = force_blend
                && self.get_model_opacity(scene, model_node, runtime_state) < 0.001;
            if let Some(mesh_group) = gpu_data.mesh_groups.get(&resource) {
                for (mesh_idx, mesh_buf) in mesh_group.iter().enumerate() {
                    let mesh_mat = self.bind_material_for_mesh(
                        gl, shader, scene, model_node,
                        res_info, mesh_idx, member_key, runtime_state, force_blend,
                    );
                    if mesh_mat.is_none() {
                        self.bind_material(gl, shader, scene, model_node, member_key, runtime_state, force_blend);
                    }
                    // A model dragged into the transparent pass by a HIDDEN mesh still
                    // has to resolve its own solid geometry against itself.
                    //
                    // Agent Free Ride's rider carries its four gadgets as meshes of the
                    // one skinned model and hides the unused ones with `shader.blend = 0`.
                    // `get_model_opacity` takes the minimum across the bound shaders, so
                    // it reports 0.000 off a mesh that draws nothing and the whole rider —
                    // body, board, jetpack — lands in PASS 2, which runs
                    // `depth_mask(false)`. With no depth inside the model the 8 meshes
                    // simply paint in index order and the torso (5..7) painted over the
                    // jetpack (1) that sits on the back, leaving only the slivers of pack
                    // falling outside the body silhouette.
                    //
                    // So for exactly that model shape — one whose classification came off
                    // a fully hidden mesh — let a mesh that is genuinely opaque (full
                    // material opacity, not additive, no alpha in its diffuse texture)
                    // write depth for its own draw, the way Director resolves it per mesh.
                    //
                    // Models with no hidden mesh keep the pass's mask untouched. The
                    // guard is deliberate scope control, not a fix for a known casualty:
                    // it keeps this out of every ordinary multi-mesh model that reaches
                    // PASS 2 for some other reason (a soft-alpha layer, an additive
                    // surface), where painting order was already the behaviour in place.
                    if force_blend && model_has_hidden_mesh {
                        let opaque_mesh = mesh_mat.as_ref().map(|mm| {
                            mm.opacity >= 0.999
                                && mm.blend_func != 1
                                && !mm.diffuse_name.is_empty()
                                && self.member_data.get(member_key).map(|g| {
                                    let n = Symbol::from_str(&mm.diffuse_name);
                                    !g.alpha_textures.contains(&n)
                                }).unwrap_or(false)
                        }).unwrap_or(false);
                        gl.depth_mask(opaque_mesh);
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
                    // renderStyle: #wire → edge lines, #point → points, else solid.
                    match Self::mesh_render_style(scene, model_node, mesh_idx, runtime_state) {
                        1 => mesh_buf.draw_wire(gl),
                        2 => mesh_buf.draw_points(gl),
                        _ => mesh_buf.draw(gl),
                    }
                    mesh_buf.unbind(gl);
                }
            } else {
                // Log missing mesh data — deduplicate by model name
                use std::sync::Mutex;
                use std::collections::HashSet;
                static LOGGED_MISS: Mutex<Option<HashSet<Symbol>>> = Mutex::new(None);
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
        // The per-mesh depth writes above are a within-model override; the
        // transparent pass owns the mask, so hand it back the way it was set.
        if force_blend {
            gl.depth_mask(false);
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
        node_name: Symbol,
        mesh_idx: Option<usize>,
    ) -> Option<&'a Symbol> {
        rs.node_shaders.get(&node_name).and_then(|m| {
            match mesh_idx {
                Some(idx) => m.get(&idx)
                    // Whole-model fallback: a `model.shaderList = shader` (or
                    // `model.shader = shader`) assignment applies to every mesh and is
                    // stored as the SOLE override at index 0. Only then does a mesh
                    // without its own entry inherit it. When the script set specific
                    // indices (`shaderList[1]`, `shaderList[2]`, …), an unset mesh keeps
                    // its DEFAULT resource shader — otherwise the LEGO minifig's legs
                    // (mesh 2, no override) inherited the head shader (mesh 0) and
                    // rendered as yellow skin instead of blue legs.
                    //
                    // `m.len() == 1` alone cannot tell `shaderList = shd` from a lone
                    // `shaderList[1] = shd`, so the indexed form is tracked explicitly:
                    // Agent Free Ride 2 voids only the enemy rider's weapon mesh
                    // (`shaderList[1] = void_mat`) and the fallback used to swallow the
                    // whole rider, leaving jet skis riding around with nobody on them.
                    .or_else(|| if m.len() == 1 && !rs.node_shaders_indexed.contains(&node_name) {
                        m.get(&0)
                    } else {
                        None
                    }),
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
        let name = Self::node_shader_override(rs, model_node.name, None)
            .copied()
            .unwrap_or(model_node.shader_name);
        if name.as_str().is_empty() { return false; }
        rs.transparent_shaders.contains(&name)
    }

    /// The `renderStyle` (0=#fill, 1=#wire, 2=#point) a given mesh of this model
    /// draws with, resolved from the effective shader for `mesh_idx`. Mirrors the
    /// shader resolution used for materials: per-mesh node override → whole-model
    /// override → the node's authored shader name. Returns 0 (#fill) when nothing
    /// set a non-fill style (the common case), so the solid path is unaffected.
    fn mesh_render_style(
        scene: &W3dScene,
        model_node: &W3dNode,
        mesh_idx: usize,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> u8 {
        let rs = match runtime_state { Some(rs) => rs, None => return 0 };
        if rs.shader_render_style.is_empty() { return 0; }
        let lookup = |name: &Symbol| -> Option<u8> {
            if name.as_str().is_empty() { return None; }
            rs.shader_render_style.get(name).copied()
        };
        // Effective shader name for this mesh, most specific first.
        let mut names: Vec<Symbol> = Vec::new();
        if let Some(n) = Self::node_shader_override(rs, model_node.name, Some(mesh_idx)) { names.push(*n); }
        if let Some(n) = Self::node_shader_override(rs, model_node.name, None) { names.push(*n); }
        if !model_node.shader_name.as_str().is_empty() { names.push(model_node.shader_name); }
        let resource = if !model_node.model_resource_name.is_empty() {
            &model_node.model_resource_name
        } else {
            &model_node.resource_name
        };
        if let Some(res_info) = scene.model_resources.get(resource) {
            for b in &res_info.shader_bindings {
                for s in &b.mesh_bindings { names.push(*s); }
            }
        }
        for n in &names {
            if let Some(st) = lookup(n) { return st; }
        }
        0
    }

    fn get_model_opacity(
        &self,
        scene: &W3dScene,
        model_node: &W3dNode,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> f32 {
        // 1. Check node-level shader override.
        //
        // Only a TRANSLUCENT result may short-circuit here. The node's authored
        // `shader_name` is `IFXModel::SetDefaultShaderID` — a FALLBACK for meshes
        // that carry no binding of their own, not an override (see IFXModel.h, and
        // the same note in docs/areazero/README.md §1). Every model node in
        // AreaZero's Level1 carries `shader = "DefaultShader"` while the real
        // material is bound PER MESH, so returning DefaultMaterial's opacity 1.0
        // here made steps 2 and 3 dead code for the whole movie: the force fields
        // in the three spawner gates, the god rays and every light glow are driven
        // to `shader.blend = 0` by `[M] 3D Shaders.BlendShader` (Director's additive
        // idiom — base contributes nothing, an `#add` layer 2 does the drawing), and
        // with opacity stuck at 1.0 the `opacity < 0.999` gate on `is_additive`
        // never opened. All of them fell into the OPAQUE pass and the gates showed
        // the bare skybox instead of the dark, speckled force field.
        //
        // Taking the MINIMUM across the bound shaders is what steps 2 and 3 already
        // do ("any transparent mesh -> whole model is transparent"); this only stops
        // step 1 from pre-empting them with an opaque answer.
        let effective_shader_name = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, model_node.name, None).copied())
            .unwrap_or(model_node.shader_name);
        if !effective_shader_name.as_str().is_empty() {
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, effective_shader_name) {
                let mat = Self::find_material_ci(&scene.materials, w3d_shader.material_name)
                    .or_else(|| Self::find_material_ci(&scene.materials, w3d_shader.name));
                if let Some(mat) = mat {
                    if mat.opacity < 0.999 {
                        return mat.opacity;
                    }
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
                    if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, *shader_name) {
                        let mat = if !w3d_shader.material_name.is_empty() {
                            Self::find_material_ci(&scene.materials, w3d_shader.material_name)
                        } else {
                            Self::find_material_ci(&scene.materials, w3d_shader.name)
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
                    if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, *shader_name) {
                        let mat = if !w3d_shader.material_name.is_empty() {
                            Self::find_material_ci(&scene.materials, w3d_shader.material_name)
                        } else {
                            Self::find_material_ci(&scene.materials, w3d_shader.name)
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
        let mut shader_names: Vec<Symbol> = Vec::new();
        if let Some(rs) = runtime_state {
            if let Some(m) = rs.node_shaders.get(&model_node.name) { shader_names.extend(m.values().copied()); }
        }
        if !model_node.shader_name.as_str().is_empty() { shader_names.push(model_node.shader_name); }
        let resource = if !model_node.model_resource_name.is_empty() { &model_node.model_resource_name } else { &model_node.resource_name };
        if let Some(res_info) = scene.model_resources.get(resource) {
            for b in &res_info.shader_bindings { for s in &b.mesh_bindings { shader_names.push(*s); } }
        }
        for shader_name in &shader_names {
            if let Some(sh) = Self::find_shader_ci(&scene.shaders, *shader_name) {
                for layer in &sh.texture_layers {
                    let lname = layer.name.to_lowercase();
                    // A loaded texture that is NOT flagged as carrying alpha = opaque cover.
                    if gpu_data.textures.contains_key(&Symbol::from_str(&lname)) && !gpu_data.alpha_textures.contains(&Symbol::from_str(&lname)) {
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
        self.model_binds_texture_in(scene, model_node, member_key, runtime_state, false)
    }

    /// True when a texture bound to this model has a smooth alpha ramp (not a
    /// binary cutout mask). Such a model must go to the blended transparent pass
    /// even at material opacity 1.0 — alpha-testing it would quantise the ramp.
    fn model_has_soft_alpha_texture(
        &self,
        scene: &W3dScene,
        model_node: &W3dNode,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        self.model_binds_texture_in(scene, model_node, member_key, runtime_state, true)
    }

    /// Shared walk of every shader that can affect `model_node`, testing whether
    /// any of its texture layers names a texture in the alpha (or soft-alpha) set.
    fn model_binds_texture_in(
        &self,
        scene: &W3dScene,
        model_node: &W3dNode,
        member_key: &(i32, i32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
        soft_only: bool,
    ) -> bool {
        let gpu_data = match self.member_data.get(member_key) {
            Some(d) => d,
            None => return false,
        };
        let set = if soft_only { &gpu_data.soft_alpha_textures } else { &gpu_data.alpha_textures };
        if set.is_empty() { return false; }

        // Collect all shader names that affect this model
        let mut shader_names: Vec<Symbol> = Vec::new();

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
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, *shader_name) {
                for layer in &w3d_shader.texture_layers {
                    if set.contains(&layer.name) {
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
    ) -> HashMap<Symbol, WebGlTexture> {
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
        let mut candidates: HashMap<Symbol, u8> = HashMap::new();
        for name in scene.texture_images.keys() {
            let lower = name.as_str().to_lowercase();
            for (i, suffix) in suffixes.iter().enumerate() {
                if lower.ends_with(suffix) {
                    let base = lower[..lower.len() - suffix.len()].to_string();
                    let entry = candidates.entry(Symbol::from_str(&base)).or_insert(0);
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
                let face_name = Symbol::from_str(&format!("{}{}", base_name, suffix));
                let face_data = scene.texture_images.iter()
                    .find(|(k, _)| **k == face_name)
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
        let targets: Vec<(Symbol, Symbol)> = runtime_state
            .map(|rs| rs.render_targets.iter().map(|(k, v)| (*k, *v)).collect())
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
                let tex_key = *tex_name;
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
                    gpu_data.texture_sizes.insert(*tex_name, (width, height));
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
uniform float u_outline_pixels;
uniform vec2 u_viewport;

out float v_facing;

void main() {
    // Model-space expansion — what the #inker SHADER TYPE's `outline_width` means.
    vec3 expanded = a_position + a_normal * u_outline_width;
    vec4 clip = u_projection * u_view * u_model * vec4(expanded, 1.0);

    // Which side of the surface this vertex is on, geometrically. The classic inverted
    // hull wants the model's FAR side, and selects it by winding (cull_face). That breaks
    // the moment a mesh is two-sided: a resource authored #both carries reverse-wound
    // copies of every face, so the NEAR surface appears as a back face too and gets drawn
    // — and because the hull is expanded OUTWARD, its fragment at a given pixel comes from
    // a point nearer the silhouette centre and is therefore NEARER than the model, so no
    // depth test can reject it. SweeTarts' bubble (#sphere, #both) filled solid white.
    // dot(N, eye→fragment) > 0 means the surface faces away from the camera, which is the
    // far side whatever the winding says.
    vec3 nv = normalize(mat3(u_view * u_model) * a_normal);
    vec3 pv = (u_view * u_model * vec4(expanded, 1.0)).xyz;
    v_facing = dot(nv, normalize(pv));

    // Screen-space expansion — what the #inker MODIFIER needs. Director's inker draws a
    // thin line of CONSTANT width; a model-space offset would instead scale with the
    // model and shrink with distance. Push the clip-space position along the projected
    // normal by a fixed number of pixels: multiplying by clip.w undoes the perspective
    // divide, so the offset survives it as an exact pixel count.
    if (u_outline_pixels > 0.0) {
        vec2 n_clip = (u_projection * vec4(nv, 0.0)).xy;
        if (dot(n_clip, n_clip) > 1e-12) {
            clip.xy += normalize(n_clip) * (u_outline_pixels * 2.0 / u_viewport) * clip.w;
        }
    }
    gl_Position = clip;
}
"#;
        let fs = r#"#version 300 es
precision mediump float;
uniform vec4 u_outline_color;
in float v_facing;
uniform float u_far_only;
out vec4 frag_color;
void main() {
    // Keep only the far side (see the note in the vertex shader) — but ONLY for the hull
    // itself. The DEPTH PREPASS runs through this same program and must record the
    // NEAREST surface; discarding the near side there left it holding the far side's
    // depth, so the hull was no longer clipped by the model and the sphere's far pole
    // showed as a white triangle at the centre of the bubble.
    if (u_far_only > 0.5 && v_facing <= 0.0) { discard; }
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
            u_outline_pixels: u("u_outline_pixels"),
            u_far_only: u("u_far_only"),
            u_viewport: u("u_viewport"),
            u_outline_color: u("u_outline_color"),
            program,
        });
        Ok(())
    }

    /// Director's #inker draws a thin line of CONSTANT width - see the reference capture
    /// of SweeTarts' level-3 bubble: a ~1px white circle around a ~130px sphere. The
    /// modifier exposes no width property at all, so this is a fixed pixel count.
    const INKER_LINE_PIXELS: f32 = 1.5;

    /// Render outlines for models carrying the #inker modifier, or wearing a shader whose
    /// TYPE is #inker.
    ///
    /// Classic inverted hull: draw the model's FAR faces expanded outward and let the
    /// model's own near surface occlude them, so only the rim survives.
    ///
    /// Two ways in: a shader whose TYPE is #inker, or a model carrying the #inker
    /// MODIFIER (`model.addModifier(#inker)`), which keeps its ordinary shader and holds
    /// its own lineColor/silhouettes/lineOffset. Only the first was ever handled - and it
    /// never fired either, since no corpus movie uses that shader type, which is how the
    /// culling bug below survived unnoticed. SweeTarts 3D's level-3 bubble is a plain
    /// #sphere whose white rim comes entirely from the modifier.
    fn render_inker_outlines(
        &mut self,
        context: &WebGL2Context,
        scene: &W3dScene,
        member_key: &(i32, i32),
        view_matrix: &[f32; 16],
        projection_matrix: &[f32; 16],
        viewport: (u32, u32),
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> Result<(), JsValue> {
        use crate::director::chunks::w3d::types::W3dShaderType;

        // (node, model-space width, screen-space width in px, colour, depth bias)
        let mut inked: Vec<(&W3dNode, f32, f32, [f32; 4], Option<f32>)> = Vec::new();
        for model_node in scene.nodes.iter().filter(|n| n.node_type == W3dNodeType::Model) {
            let ink = runtime_state.and_then(|rs| rs.inker_state.get(&model_node.name));
            let shader_name = runtime_state
                .and_then(|rs| Self::node_shader_override(rs, model_node.name, None).copied())
                .unwrap_or(model_node.shader_name);
            let inker_shader = match Self::find_shader_ci(&scene.shaders, shader_name) {
                Some(s) if s.shader_type == W3dShaderType::Inker => Some(s),
                _ => None,
            };
            // `silhouettes` outlines the model's border and `boundary` the edge of an open
            // surface; both are what this hull approximates. `creases` needs per-edge
            // dihedral-angle detection and has no implementation - a model with ONLY
            // creases on draws nothing rather than a wrong silhouette.
            let modifier_draws = ink.map(|i| i.silhouettes || i.boundary).unwrap_or(false);
            match (ink, inker_shader) {
                (Some(i), _) if modifier_draws => {
                    let color = [i.line_color.0 as f32 / 255.0, i.line_color.1 as f32 / 255.0,
                                 i.line_color.2 as f32 / 255.0, 1.0];
                    // `lineOffset` is documented as "where lines are drawn relative to the
                    // surface being shaded and the camera" - a depth bias, NOT a width, and
                    // one that applies only while `useLineOffset` is TRUE. Reading it as a
                    // hull thickness scaled the line by an arbitrary movie value (this
                    // movie's -10 on a radius-25 sphere came out a 0.4% hairline).
                    let bias = if i.use_line_offset { Some(i.line_offset) } else { None };
                    inked.push((model_node, 0.0, Self::INKER_LINE_PIXELS, color, bias));
                }
                (_, Some(s)) => {
                    let w = if s.outline_width > 0.0 { s.outline_width } else { 0.02 };
                    inked.push((model_node, w, 0.0, s.outline_color, None));
                }
                _ => {}
            }
        }
        if inked.is_empty() { return Ok(()); }

        self.ensure_outline_shader(context)?;
        let gl = context.gl();
        let outline = self.outline_shader.as_ref().unwrap();

        gl.use_program(Some(&outline.program));
        gl.uniform_matrix4fv_with_f32_array(outline.u_view.as_ref(), false, view_matrix);
        gl.uniform_matrix4fv_with_f32_array(outline.u_projection.as_ref(), false, projection_matrix);
        gl.uniform2f(outline.u_viewport.as_ref(), viewport.0.max(1) as f32, viewport.1.max(1) as f32);

        gl.disable(WebGl2RenderingContext::BLEND);
        gl.enable(WebGl2RenderingContext::DEPTH_TEST);
        gl.depth_func(WebGl2RenderingContext::LEQUAL);
        gl.enable(WebGl2RenderingContext::CULL_FACE);

        for (model_node, model_w, px, color, bias) in inked {
            let world_matrix = self.accumulate_transform_with_state(scene, model_node, runtime_state);
            gl.uniform_matrix4fv_with_f32_array(outline.u_model.as_ref(), false, &world_matrix);
            gl.uniform4f(outline.u_outline_color.as_ref(), color[0], color[1], color[2], color[3]);

            let resource = if !model_node.model_resource_name.is_empty() {
                &model_node.model_resource_name
            } else {
                &model_node.resource_name
            };
            let has_group = self.member_data.get(member_key)
                .map(|d| d.mesh_groups.contains_key(resource)).unwrap_or(false);
            if !has_group { continue }


            // PASS A - depth only. The hull is clipped by the model itself, and a
            // TRANSLUCENT model never wrote any depth: the transparent pass draws with
            // `depth_mask(false)`, so the far hull passed the depth test everywhere and
            // painted the bubble as a solid white disc. Lay the model's near surface into
            // the depth buffer first, writing no colour. This whole pass runs after the
            // particles for exactly this reason - the extra depth must not occlude
            // anything still to be drawn.
            gl.color_mask(false, false, false, false);
            gl.depth_mask(true);
            // Culling OFF, not cull_face(FRONT): a resource authored #both (SweeTarts'
            // bubble) carries reverse-wound inner faces, so front-culling no longer
            // isolates the near surface and the prepass laid down the FAR depth - the
            // hull stopped being clipped and the bubble went back to a solid white disc.
            // Depth-only with the depth test does the right thing for one- and two-sided
            // meshes alike: whatever the winding, the buffer keeps the nearest fragment.
            gl.disable(WebGl2RenderingContext::CULL_FACE);
            gl.uniform1f(outline.u_outline_width.as_ref(), 0.0);
            gl.uniform1f(outline.u_outline_pixels.as_ref(), 0.0);
            gl.uniform1f(outline.u_far_only.as_ref(), 0.0);
            self.draw_mesh_group(gl, member_key, resource);

            // PASS B - the expanded hull's FAR faces.
            //
            // This pass used to `cull_face(FRONT)` "to draw back faces expanded". But this
            // renderer's projection is Y-flipped, so cull_face(FRONT)/front_face(CCW) is
            // already its global default for ORDINARY geometry (see the camera setup, and
            // `visibility = #back` - draw only far faces - implemented as cull_face(BACK)).
            // The pass was therefore drawing the model's NEAR faces expanded outward: in
            // front of the model at every pixel, a solid disc no matter what the depth
            // buffer held, which is why a depth prepass and an explicit DEPTH_TEST both
            // changed nothing on their own. The far side is BACK here.
            gl.color_mask(true, true, true, true);
            gl.depth_mask(false);
            // No culling: the shader's N.V discard selects the far side, which is correct
            // whether or not the mesh carries reverse-wound duplicates.
            gl.disable(WebGl2RenderingContext::CULL_FACE);
            // LESS, not LEQUAL. The hull is expanded in SCREEN space, which does not
            // change its depth, so a face coincident with the one the prepass recorded
            // must be REJECTED - otherwise a two-sided mesh's reverse-wound near faces
            // (which cull_face(BACK) also selects) pass at exactly the prepass depth and
            // fill the silhouette solid. Only geometry genuinely in front of what the
            // depth buffer holds - i.e. the rim, outside the model - should draw.
            gl.depth_func(WebGl2RenderingContext::LESS);
            gl.uniform1f(outline.u_outline_width.as_ref(), model_w);
            gl.uniform1f(outline.u_outline_pixels.as_ref(), px);
            gl.uniform1f(outline.u_far_only.as_ref(), 1.0);
            if let Some(units) = bias {
                gl.enable(WebGl2RenderingContext::POLYGON_OFFSET_FILL);
                gl.polygon_offset(0.0, units);
            }
            self.draw_mesh_group(gl, member_key, resource);
            if bias.is_some() {
                gl.disable(WebGl2RenderingContext::POLYGON_OFFSET_FILL);
                gl.polygon_offset(0.0, 0.0);
            }
        }

        gl.depth_mask(true);
        gl.depth_func(WebGl2RenderingContext::LEQUAL); // the renderer's default
        gl.cull_face(WebGl2RenderingContext::FRONT); // back to the Y-flipped default
        Ok(())
    }

    /// Draw every mesh of a model resource with whatever program/state is already bound.
    fn draw_mesh_group(&self, gl: &WebGl2RenderingContext, member_key: &(i32, i32), resource: &Symbol) {
        if let Some(mesh_group) = self.member_data.get(member_key)
            .and_then(|d| d.mesh_groups.get(resource))
        {
            for mesh_buf in mesh_group {
                mesh_buf.bind(gl);
                mesh_buf.draw(gl);
                mesh_buf.unbind(gl);
            }
        }
    }

    /// Director's red/white checkerboard is the placeholder for a shader that has NO
    /// texture on it AT ALL — it is what a freshly created primitive shows until something
    /// is put on it. A shader carrying ANY texture layer is textured, even when that layer
    /// is not the diffuse one.
    ///
    /// SweeTarts 3D's level-3 mascot is the case that exposed this: a `#sphere` shader with
    /// `texture = VOID` and a "transcrome" REFLECTION MAP on layer 3. The reflection map is
    /// applied by a separate pass, so the diffuse scan found nothing, the primitive fallback
    /// fired, and the bubble rendered as an opaque red/white checker sphere instead of a
    /// translucent bubble.
    fn shader_has_any_texture(shader: &W3dShader) -> bool {
        shader.texture_layers.iter().any(|l| !l.name.is_empty())
    }

    /// Case-insensitive shader lookup (W3D files have inconsistent casing).
    fn find_shader_ci<'a>(shaders: &'a [W3dShader], name: Symbol) -> Option<&'a W3dShader> {
        shaders.iter().find(|s| s.name == name)
    }

    /// Case-insensitive material lookup.
    fn find_material_ci<'a>(materials: &'a [W3dMaterial], name: Symbol) -> Option<&'a W3dMaterial> {
        materials.iter().find(|m| m.name == name)
    }

    /// Find the first shader that references a material by name.
    fn find_shader_for_material_ci<'a>(scene: &'a W3dScene, material_name: Symbol) -> Option<&'a W3dShader> {
        scene.shaders.iter().find(|s| s.material_name == material_name)
    }

    /// Resolve a candidate name to a shader, allowing either shader names or material names.
    fn resolve_shader_candidate_ci<'a>(scene: &'a W3dScene, candidate: Symbol) -> Option<&'a W3dShader> {
        Self::find_shader_ci(&scene.shaders, candidate)
            .or_else(|| Self::find_shader_for_material_ci(scene, candidate))
    }

    /// Resolve a candidate name to a material, allowing either material names or shader names.
    fn resolve_material_candidate_ci<'a>(scene: &'a W3dScene, candidate: Symbol) -> Option<&'a W3dMaterial> {
        Self::find_material_ci(&scene.materials, candidate)
            .or_else(|| {
                Self::find_shader_ci(&scene.shaders, candidate)
                    .and_then(|s| Self::find_material_ci(&scene.materials, s.material_name))
            })
    }

    /// Resolve all texture layers for a shader: diffuse, extra blend layers, and specular map.
    /// Categorizes layers by tex_mode: 0/5 = diffuse, 6 = specular, others = diffuse.
    /// Extra layers (beyond the first diffuse) are returned with proper blend modes.
    fn find_texture_layers<'a>(
        layers: &[crate::director::chunks::w3d::types::W3dTextureLayer],
        gpu_data: &'a MemberGpuData,
        shader_type: W3dShaderType,
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
            diffuse_name: String::new(),
        };

        let mut diffuse_name = String::new();

        // A #normalMap shader fixes its layer roles by slot: 1 = normal map,
        // 2 = diffuse, 3 = specular. Tangent-space normal mapping isn't
        // implemented, so drop slot 1 rather than let the generic "first usable
        // layer wins" rule paint the model with its blue/purple normal map, and
        // route slot 3 to the specular slot the standard path already honors.
        let normal_map_shader = shader_type == W3dShaderType::NormalMap;

        for (layer_idx, layer) in layers.iter().enumerate() {
            if layer.name.is_empty() { continue; }
            if normal_map_shader && layer_idx == 0 { continue; }
            let lower = layer.name.as_str().to_lowercase();
            let tex = gpu_data.textures.get(&layer.name);
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
            if layer.tex_mode == 6 || (normal_map_shader && layer_idx == 2) {
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
            // A baked lighting layer is one that ACCOMPANIES a diffuse, never the
            // only texture on the shader. The name test alone misfires whenever a
            // model's actual surface art happens to be called "...Shadow..." —
            // AreaZero's `MenuPlayerShadow_Material` binds a single texture,
            // `MenuPlayerShadow_Texture`, which is the blob-shadow/decal ATLAS for
            // the player shadow, bullet holes and blast marks. Rejecting it left
            // `result.diffuse` empty, so `bind_texture_layers` returned false, the
            // caller fell through to its material-only path with
            // `u_has_texture = 0`, and the quads rendered as flat WHITE mats on the
            // floor with the decals missing entirely.
            //
            // Require corroboration from position: only a layer AFTER the first
            // usable one can be baked lighting. Phosphor's lightmap sits in a later
            // slot, so its case is unaffected; a lone texture is always the diffuse.
            let is_baked_lighting = (lower.contains("lightmap") || lower.contains("shadow"))
                && (result.diffuse.is_some() || layers.iter().skip(layer_idx + 1).any(|l| {
                    !l.name.is_empty() && gpu_data.textures.contains_key(&l.name)
                }));
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

                // IFX blend func (IFXEnums.h): 0 = IFX_SELECT_ARG0, 1 = IFX_ADD,
                // 2 = IFX_MODULATE (out = tex * incoming), 3 = IFX_INTERPOLATE.
                // Mapped to our extra-layer modes below (1 = multiply, 2 = add).
                // Lightmap-only meshes (empty textureList[1], lightmap in
                // textureList[2]) shade as material colour multiplied by light
                // intensity, and have no diffuse layer to read a blend function
                // against, so they are pinned to multiply.
                let lightmap_only = lower.contains("lightmap") && !lower.contains("shadow")
                    && layer_idx > 0
                    && layers[..layer_idx].iter().all(|prev| prev.name.is_empty());
                let blend = if lightmap_only {
                    1
                } else {
                    // Otherwise honour the AUTHORED blend function. A name-based
                    // "lightmaps composite additively" override used to sit here and
                    // is backwards for the ordinary diffuse+lightmap pair: baked
                    // light MULTIPLIES the diffuse (IFX blend func 2 = IFX_MODULATE,
                    // and AreaZero's four Hangar* shaders all report #multiply on
                    // every layer). Adding it blew out lit surfaces and, worse, left
                    // UNLIT geometry at full diffuse brightness — the pale metalwork
                    // in the spawner-gate shafts where the reference frame has a
                    // dark, blue-speckled void.
                    match layer.blend_func {
                        1 => 2,  // IFX_ADD → our add mode
                        2 => 1,  // IFX_MODULATE → our multiply mode
                        _ => 1,  // default multiply
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

        result.diffuse_name = diffuse_name;
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
    ) -> Option<Symbol> {
        // 1) Runtime override (model.shader = s1 / shaderList[1] = ref)
        if let Some(name) = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, model_node.name, None))
        {
            return Some(*name);
        }
        // 2) Model-resource first-mesh shader binding (prefer non-DefaultShader)
        let resource = if !model_node.model_resource_name.is_empty() {
            &model_node.model_resource_name
        } else {
            &model_node.resource_name
        };
        if let Some(res) = scene.model_resources.get(resource) {
            let mut fallback: Option<Symbol> = None;
            for binding in &res.shader_bindings {
                if !binding.mesh_bindings.is_empty() && !binding.mesh_bindings[0].as_str().is_empty() {
                    let name = binding.mesh_bindings[0];
                    if name != BuiltInSymbol::DefaultShader {
                        return Some(name);
                    } else if fallback.is_none() {
                        fallback = Some(name);
                    }
                }
            }
            if fallback.is_some() { return fallback; }
        }
        // 3) Node's shader_name
        if !model_node.shader_name.as_str().is_empty() {
            return Some(model_node.shader_name);
        }
        // 4) Model index → shader index
        let mi = scene.nodes.iter()
            .filter(|n| n.node_type == W3dNodeType::Model)
            .position(|n| n.name == model_node.name);
        if let Some(mi) = mi {
            if mi < scene.shaders.len() {
                return Some(scene.shaders[mi].name);
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
        let refl = Self::find_shader_ci(&scene.shaders, shader_name)
            .and_then(|sh| sh.texture_layers.iter()
                .find(|l| l.tex_mode == 4 && !l.name.is_empty())
                .map(|l| (l.name.clone(), l.blend_const, l.blend_func)));
        let (tex_name, blend_const, blend_func) = match refl { Some(x) => x, None => return };
        // The layer's blendFunctionList entry decides how the reflection composites
        // (Director 11.5 Scripting Dictionary, `blendFunctionList`). Treating every
        // reflection as #blend put Agent Free Ride's coins — an #add gold env map
        // over a lettered ring — at a 50/50 mix with the env map, which read as a
        // featureless white blob instead of a coin.
        // File encoding: 0 = #replace, 1 = #add, 2 = #multiply, 3 = #blend.
        let blend_mode = match blend_func {
            0 => 8, // #replace
            1 => 6, // #add
            2 => 7, // #multiply
            _ => 5, // #blend — ratio from blendConstant
        };
        let gpu_data = match self.member_data.get(member_key) { Some(d) => d, None => return };
        let tex = match gpu_data.textures.get(&Symbol::from_str(&tex_name.to_lowercase())) {
            Some(t) => t,
            None => return,
        };
        gl.active_texture(WebGl2RenderingContext::TEXTURE2);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_S, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_WRAP_T, WebGl2RenderingContext::CLAMP_TO_EDGE as i32);
        gl.uniform1i(shader.u_layer2_blend.as_ref(), blend_mode);
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
            .and_then(|rs| Self::node_shader_override(rs, model_node.name, None).copied())
            .unwrap_or(model_node.shader_name);

        if !effective_shader_name.as_str().is_empty() {
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, Symbol::from_str(effective_shader_name.as_str())) {
                // Find material: try shader's material_name, then shader name itself
                let mat = if !w3d_shader.material_name.is_empty() {
                    Self::find_material_ci(&scene.materials, w3d_shader.material_name)
                } else { None }
                    .or_else(|| Self::find_material_ci(&scene.materials, w3d_shader.name));
                if let Some(mat) = mat {
                    self.set_material_uniforms(gl, shader, mat);
                    mat_found = true;
                }

                // Bind texture layers
                if let Some(gpu_data) = self.member_data.get(member_key) {
                    let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data, w3d_shader.shader_type);
                    tex_bound = Self::bind_texture_layers(gl, shader, &layers);
                }
            }
        }

        // Fall back to the model resource's shader bindings when the node's own
        // shader produced no MATERIAL *or* no TEXTURE.
        //
        // Gating this on `!mat_found` alone was wrong: a node whose shader_name is
        // "DefaultShader" resolves DefaultMaterial, so mat_found went true and the
        // fallback was skipped even though nothing textured the model. Director
        // binds the real material through the resource's per-mesh bindings, and
        // AreaZero leaves the node field at "DefaultShader" for its whole
        // MenuCharacter cast — PlayerShadow001-004, BulletHole*, BlastMark* all
        // drew as untextured white quads on the floor.
        //
        // Also try EVERY binding group, not just the first: the first is usually
        // the generic "default"/DefaultShader group, with the model's real
        // material in a later one. DefaultShader is therefore considered last.
        if !mat_found || !tex_bound {
            let resource = if !model_node.model_resource_name.is_empty() {
                &model_node.model_resource_name
            } else {
                &model_node.resource_name
            };
            if let Some(res_info) = scene.model_resources.get(resource) {
                let mut candidates: Vec<Symbol> = Vec::new();
                for binding in &res_info.shader_bindings {
                    for mb in binding.mesh_bindings.iter().filter(|b| !b.is_empty()) {
                        candidates.push(*mb);
                    }
                    if !binding.name.is_empty() {
                        candidates.push(binding.name);
                    }
                }
                // Stable partition: specific shaders first, DefaultShader last.
                candidates.sort_by_key(|c| c.eq_ignore_ascii_case("DefaultShader"));

                for cand in &candidates {
                    if tex_bound {
                        break;
                    }
                    let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, *cand) else { continue };
                    let bound = if let Some(gpu_data) = self.member_data.get(member_key) {
                        let layers = Self::find_texture_layers(
                            &w3d_shader.texture_layers, gpu_data, w3d_shader.shader_type,
                        );
                        Self::bind_texture_layers(gl, shader, &layers)
                    } else {
                        false
                    };
                    if bound {
                        tex_bound = true;
                        // The shader that supplied the texture owns the material too —
                        // otherwise the model keeps DefaultMaterial's colours.
                        if let Some(mat) = Self::find_material_ci(&scene.materials, w3d_shader.material_name)
                            .or_else(|| Self::find_material_ci(&scene.materials, w3d_shader.name))
                        {
                            self.set_material_uniforms(gl, shader, mat);
                            mat_found = true;
                        }
                    } else if !mat_found {
                        if let Some(mat) = Self::find_material_ci(&scene.materials, w3d_shader.material_name) {
                            self.set_material_uniforms(gl, shader, mat);
                            mat_found = true;
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
        let w3d_shader_opt = Self::find_shader_ci(&scene.shaders, Symbol::from_str(effective_shader_name.as_str()));
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
            .and_then(|s| Self::find_material_ci(&scene.materials, s.material_name))
            .map(|m| m.opacity)
            .unwrap_or(1.0);
        Self::apply_blend_mode(gl, shader, opacity, first_blend_func, force_blend);
    }

    /// Get the first texture layer's blend_func for a model node
    fn get_first_blend_func(&self, scene: &W3dScene, node: &W3dNode, runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>) -> u8 {
        // Consider EVERY shader that can affect this model, not just the node's
        // own `shader_name`. Director binds a material through the model
        // RESOURCE's per-mesh shader bindings; the node field is frequently left
        // at "DefaultShader" (all of AreaZero's MenuCharacter FX are like this).
        // Reading only the node field returned DefaultShader's blend func, so an
        // additive surface never reached the additive branch of
        // `apply_blend_mode` — and with `shader.blend = 0` the draw was then
        // multiplied to nothing, which is why the muzzle flash, bullet streaks
        // and sparks stayed invisible even once the pass routing was right.
        let mut names: Vec<Symbol> = Vec::new();
        if let Some(rs) = runtime_state {
            if let Some(over) = Self::node_shader_override(rs, node.name, None) {
                names.push(*over);
            }
        }
        if !node.shader_name.is_empty() {
            names.push(node.shader_name);
        }
        let resource = if !node.model_resource_name.is_empty() {
            &node.model_resource_name
        } else {
            &node.resource_name
        };
        if let Some(res_info) = scene.model_resources.get(resource) {
            for binding in &res_info.shader_bindings {
                names.extend(binding.mesh_bindings.iter().cloned());
            }
        }
        // An #add layer on ANY contributing shader wins — the surface is additive.
        let mut first = 0u8;
        let mut seen = false;
        for n in names.iter().filter(|n| !n.is_empty()) {
            if let Some(sh) = Self::find_shader_ci(&scene.shaders, *n) {
                // Classifier path: opacity is applied by the caller's own
                // `is_additive` gate, so leave the promotion unconditional here.
                let bf = Self::effective_blend_func(sh, 0.0);
                if bf == 1 {
                    return 1;
                }
                if !seen {
                    first = bf;
                    seen = true;
                }
            }
        }
        first
    }

    /// The blend function that actually decides how a shader composites.
    ///
    /// Normally that is texture layer 1's, but Director's standard ADDITIVE
    /// idiom puts the additive contribution on a LATER layer. `[M] 3D Shaders`
    /// `BlendShader` builds it like this:
    ///
    /// ```lingo
    /// tShader.blend = 0.0                    -- base contributes nothing
    /// tShader.blendFunctionList[1] = #blend
    /// tShader.blendConstantList[1] = 0.0
    /// tShader.textureList[2] = tShader.textureList[1]
    /// tShader.blendFunctionList[2] = #add    -- SAME texture, added on top
    /// ```
    ///
    /// Reading only layer 1 saw `#blend` at constant 0 and drew nothing, so
    /// AreaZero's muzzle flash, bullet streaks, sparks and smoke never appeared
    /// (defect 3.2). If ANY layer is `#add` (IFX blend func 1) the surface is
    /// additive.
    /// The blend function the SURFACE composites with. `opacity` is the material
    /// opacity for this draw; pass 0.0 where it isn't known and the additive
    /// promotion should stay unconditional.
    fn effective_blend_func(
        shader: &crate::director::chunks::w3d::types::W3dShader,
        opacity: f32,
    ) -> u8 {
        // An `#add` layer only makes the whole SURFACE additive in Director's
        // additive-FX idiom, where the layers UNDERNEATH contribute nothing: the
        // base layer is `#blend` at constant 0 (AreaZero's MenuCharacter FX) or
        // the material is driven to zero opacity. When the base layer is a real
        // diffuse — `#replace` or `#multiply` over an actual texture — the `#add`
        // layer is a light-ADD MAP that combines with the layers below it INSIDE
        // the material, exactly as `apply_fog`'s siblings do per fragment.
        //
        // Burnin' Rubber's garage is the case that separates the two: its
        // `AssignTexture` builds `[1] #replace` (the concrete diffuse),
        // `[2] #add` (GarageLightmapAdd) and `[3] #multiply` (the lightmap) at
        // full opacity. Promoting that to a framebuffer-additive surface drew the
        // whole showroom — and every car — as a washed-out white haze over the
        // camera clear.
        if shader.texture_layers.iter().any(Self::layer_forces_additive)
            && (opacity < 0.999 || !Self::base_layer_contributes(shader))
        {
            return 1;
        }
        shader.texture_layers.first().map(|l| l.blend_func).unwrap_or(0)
    }

    /// Whether the shader's FIRST texture layer puts any colour on the surface.
    /// `#blend` (3) at a blend constant of 0 is Director's "base contributes
    /// nothing" spelling — the additive idiom's marker.
    fn base_layer_contributes(shader: &crate::director::chunks::w3d::types::W3dShader) -> bool {
        match shader.texture_layers.first() {
            None => false,
            Some(l) => !(l.blend_func == 3 && l.blend_const.abs() <= 0.001),
        }
    }

    /// Whether a texture layer makes the whole SURFACE composite additively
    /// against the frame buffer.
    ///
    /// An `#add` layer normally does — that is Director's additive-FX idiom, where
    /// `shader.blend` is left at 0 and the `#add` sits on a later layer. But a
    /// `#reflection` layer (tex_mode 4) is different: its blend function says how
    /// the ENVIRONMENT MAP combines with the surface underneath it, not how the
    /// surface combines with what is already on screen. Agent Free Ride's coins are
    /// exactly that shape — an opaque lettered ring plus an `#add` gold env map —
    /// and treating the model as additive drew each coin as a saturated white blob
    /// over the bright sky instead of a coin. The reflection's own contribution is
    /// applied per-fragment in `apply_reflection_map`.
    fn layer_forces_additive(l: &crate::director::chunks::w3d::types::W3dTextureLayer) -> bool {
        l.blend_func == 1 && l.tex_mode != 4
    }

    /// True when a model composites additively — its shader carries an `#add`
    /// texture layer. Such a model must reach the blended pass regardless of
    /// `shader.blend`, which the additive idiom deliberately sets to 0.
    fn model_is_additive(
        &self,
        scene: &W3dScene,
        node: &W3dNode,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        let mut names: Vec<Symbol> = Vec::new();
        if let Some(rs) = runtime_state {
            if let Some(map) = rs.node_shaders.get(&node.name) {
                names.extend(map.values().cloned());
            }
        }
        if !node.shader_name.is_empty() {
            names.push(node.shader_name);
        }
        let resource = if !node.model_resource_name.is_empty() {
            &node.model_resource_name
        } else {
            &node.resource_name
        };
        if let Some(res_info) = scene.model_resources.get(resource) {
            for binding in &res_info.shader_bindings {
                names.extend(binding.mesh_bindings.iter().cloned());
            }
        }
        names.iter().any(|n| {
            Self::find_shader_ci(&scene.shaders, *n)
                .map(|s| s.texture_layers.iter().any(Self::layer_forces_additive))
                .unwrap_or(false)
        })
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
    ) -> Option<MeshMatInfo> {
        // Check per-mesh shader override first (from Lingo shaderList[I] = shaderRef)
        if let Some(override_name) = runtime_state
            .and_then(|rs| Self::node_shader_override(rs, model_node.name, Some(mesh_idx)))
        {
            if let Some(w3d_shader) = Self::find_shader_ci(&scene.shaders, Symbol::from_str(override_name.as_str())) {
                let mat = if !w3d_shader.material_name.is_empty() {
                    Self::find_material_ci(&scene.materials, w3d_shader.material_name)
                } else { None }
                    .or_else(|| Self::find_material_ci(&scene.materials, w3d_shader.name));
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
                let mut diffuse_name = String::new();
                if let Some(gpu_data) = self.member_data.get(member_key) {
                    let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data, w3d_shader.shader_type);
                    has_lightmap_layer = !layers.extra_layers.is_empty();
                    diffuse_name = layers.diffuse_name.clone();
                    tex_bound = Self::bind_texture_layers(gl, shader, &layers);
                }
                let is_prim = res_info.and_then(|r| r.primitive_type.as_ref()).is_some();
                if !tex_bound && is_prim && !Self::shader_has_any_texture(w3d_shader) {
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
                } else if !tex_bound {
                    // An untextured, non-primitive mesh on the OVERRIDE path left
                    // `u_has_texture` at whatever the previous draw set, so it
                    // sampled a stale texture belonging to another model. The
                    // non-override path below already writes the 0 explicitly;
                    // this branch simply did not exist. A uniform set only on some
                    // paths is the same stale-uniform trap `u_flat_shading` and
                    // `u_projection` hit before in this renderer.
                    gl.uniform1i(shader.u_has_texture.as_ref(), 0);
                }
                // IFX default: white diffuse for textured models unless useDiffuseWithTexture
                if tex_bound && !w3d_shader.use_diffuse_with_texture {
                    gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
                }
                // effective_blend_func, not layer 0: Director's additive idiom puts
                // the #add on a LATER layer (layer 0 is #blend at constant 0).
                let opacity = mat.map(|m| m.opacity).unwrap_or(1.0);
                let first_bf = Self::effective_blend_func(w3d_shader, opacity);
                Self::apply_blend_mode(gl, shader, opacity, first_bf, force_blend);
                return Some(MeshMatInfo { opacity, blend_func: first_bf, diffuse_name });
            }
        }

        let res_info = match res_info {
            Some(r) => r,
            None => return None,
        };

        // Per-mesh shader candidates, SPECIFIC ONES FIRST.
        //
        // A resource usually carries two bindings: an auto-generated "default"
        // one whose mesh entries are all DefaultShader, and the real one naming
        // the mesh's actual shader (Rasterwerks dm_acheron_bluffs:
        // `default=[DefaultShader,…]` and `_light_bunker=[si_fi_28_30C,…]`).
        // `shader_bindings` lists default FIRST, and the loop below returns on
        // the first candidate that binds a texture — so DefaultShader won every
        // time it happened to hold textures and the mesh's real shader was never
        // reached, leaving the whole map rendering with DefaultShader's
        // leftovers (no VMtl/si_fi shader ever reached find_texture_layers).
        //
        // Same set as before, just ordered: DefaultShader entries go last so
        // they still act as the fallback they were meant to be. Mirrors the
        // "skip DefaultShader as best_material when there are more specific
        // candidates" rule applied to material selection below.
        let mut candidate_names: Vec<Symbol> = Vec::new();
        let mut default_candidates: Vec<Symbol> = Vec::new();
        for binding in &res_info.shader_bindings {
            if mesh_idx < binding.mesh_bindings.len() && !binding.mesh_bindings[mesh_idx].is_empty() {
                let name = binding.mesh_bindings[mesh_idx].clone();
                if name == BuiltInSymbol::DefaultShader
                    || binding.name.as_str().eq_ignore_ascii_case("default")
                {
                    default_candidates.push(name);
                } else {
                    candidate_names.push(name);
                }
            }
        }
        candidate_names.extend(default_candidates);

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
            .and_then(|rs| Self::node_shader_override(rs, model_node.name, Some(mesh_idx)))
            .cloned()
            .unwrap_or_else(|| Symbol::from_str(&model_node.shader_name.clone().to_string()));
        if !effective_shader_name.as_str().is_empty() {
            candidate_names.push(Symbol::from_str(&effective_shader_name.as_str()));
        }

        for binding in &res_info.shader_bindings {
            if !binding.name.is_empty() {
                candidate_names.push(binding.name.clone());
            }
        }

        // DefaultShader last. It is the generic fallback every exporter writes into
        // the "default" binding group, and it usually carries a DefaultTexture — so
        // when it is tried FIRST it binds that placeholder and wins, and the model's
        // real material is never reached. The `best_material` selection below
        // already guards against DefaultShader; the TEXTURE search did not, so a
        // member whose DefaultTexture happens to have image data rendered entirely
        // in the placeholder. AreaZero's Level1 does exactly that — the whole hangar
        // drew flat white while passes from other members on the same sprite (the
        // FPS weapon view) looked correct.
        //
        // Stable sort: everything else keeps its authored order, DefaultShader moves
        // to the end, and it still wins when it is the only candidate.
        candidate_names.sort_by_key(|c| c.eq_ignore_ascii_case("DefaultShader"));

        let mut best_material: Option<&W3dMaterial> = None;
        let mut best_blend_func = 0u8;

        for candidate in &candidate_names {
            if candidate.is_empty() {
                continue;
            }

            let w3d_shader = Self::resolve_shader_candidate_ci(scene, *candidate);
            let mat = Self::resolve_material_candidate_ci(scene, *candidate)
                .or_else(|| {
                    w3d_shader.and_then(|s| {
                        if !s.material_name.is_empty() {
                            Self::find_material_ci(&scene.materials, s.material_name)
                        } else {
                            None
                        }
                    })
                })
                .or_else(|| w3d_shader.and_then(|s| Self::find_material_ci(&scene.materials, s.name)));

            // Skip DefaultShader as best_material when there are more specific candidates.
            // DefaultShader often has white default material that overrides model-specific
            // materials (e.g., cloned models with yellow emissive from their source member).
            if best_material.is_none() && !(*candidate == BuiltInSymbol::DefaultShader && candidate_names.len() > 1) {
                best_material = mat;
                // See effective_blend_func: an #add layer anywhere makes the
                // surface additive, and it is never layer 0 in Director's idiom.
                best_blend_func = w3d_shader
                    .map(|s| Self::effective_blend_func(s, mat.map(|m| m.opacity).unwrap_or(1.0)))
                    .unwrap_or(0);
            }

            let mut tex_bound = false;
            let mut diffuse_name = String::new();
            if let (Some(gpu_data), Some(w3d_shader)) = (self.member_data.get(member_key), w3d_shader) {
                let layers = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data, w3d_shader.shader_type);
                diffuse_name = layers.diffuse_name.clone();
                tex_bound = Self::bind_texture_layers(gl, shader, &layers);
            }

            if tex_bound {
                if let Some(m) = mat {
                    self.set_material_uniforms(gl, shader, m);
                }
                // `shader.flat`. Written on EVERY mesh draw, never only when
                // true: this program is shared, and a uniform left set by the
                // previous draw is exactly how a stale-uniform bug starts.
                gl.uniform1i(
                    shader.u_flat_shading.as_ref(),
                    if w3d_shader.map(|s| s.flat).unwrap_or(false) { 1 } else { 0 },
                );
                // IFX default: white diffuse for textured models unless useDiffuseWithTexture
                let use_diffuse = w3d_shader.map(|s| s.use_diffuse_with_texture).unwrap_or(false);
                if !use_diffuse {
                    gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
                }
                // effective_blend_func, not layer 0. This is the call that actually
                // decides how AreaZero's MenuCharacter FX composite: their material
                // is bound per-mesh through the model resource, and layer 0 is
                // `#blend` at constant 0 with the `#add` on layer 1. Reading layer 0
                // gave 3, so the additive branch never ran and the muzzle flash,
                // bullet streaks and sparks drew nothing.
                let opacity = mat.map(|m| m.opacity).unwrap_or(1.0);
                let first_bf = w3d_shader
                    .map(|s| Self::effective_blend_func(s, opacity))
                    .unwrap_or(0);
                Self::apply_blend_mode(gl, shader, opacity, first_bf, force_blend);
                return Some(MeshMatInfo { opacity, blend_func: first_bf, diffuse_name });
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
        // Same rule as the override path: a shader that carries any texture layer — a
        // reflection map included — is textured, and must not get the placeholder.
        let inked_by_any_layer = candidate_names.iter().any(|n| {
            Self::resolve_shader_candidate_ci(scene, *n)
                .map(Self::shader_has_any_texture)
                .unwrap_or(false)
        });
        let is_primitive = res_info.primitive_type.is_some() && !inked_by_any_layer;
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
            return Some(MeshMatInfo {
                opacity: mat.opacity,
                blend_func: best_blend_func,
                diffuse_name: String::new(),
            });
        }

        None
    }

    fn set_material_uniforms(&self, gl: &WebGl2RenderingContext, shader: &Shader3d, mat: &W3dMaterial) {
        gl.uniform4f(shader.u_diffuse_color.as_ref(), mat.diffuse[0], mat.diffuse[1], mat.diffuse[2], mat.diffuse[3]);
        gl.uniform4f(shader.u_ambient_color.as_ref(), mat.ambient[0], mat.ambient[1], mat.ambient[2], mat.ambient[3]);
        gl.uniform4f(shader.u_emissive_color.as_ref(), mat.emissive[0], mat.emissive[1], mat.emissive[2], mat.emissive[3]);
        // `reflectivity` is a REFLECTANCE, not a Phong exponent.
        //
        // 11.5 dictionary: `reflectivity` is "the percentage of light to be
        // reflected off the surface of a model", 0.0-100.0, default 0.0 — while
        // `shininess` is a separate property, "the percentage of shader surface
        // devoted to highlights", 0-100, default 30. The two are not the same
        // quantity, and neither of them is the exponent `pow(N·H, e)` wants.
        //
        // Feeding reflectivity in as that exponent inverted its meaning: the LESS
        // reflective the material, the SMALLER the exponent and so the BROADER the
        // highlight. Burnin' Rubber 3's menu bars are the visible case —
        // "Orange_Material" is diffuse/emissive orange with WHITE specular and
        // reflectivity 0.03, i.e. essentially matte. That became `pow(N·H, 3.0)`,
        // which is ~1 across the whole quad, so a full-strength white highlight was
        // added over every pixel and the solid orange "WORLD DOMINATION" bar (and
        // the challenge/cash panels behind it) rendered pale cream.
        //
        // So: a material that states a real `shininess` keeps using it as before;
        // otherwise reflectivity scales the specular CONTRIBUTION and the exponent
        // falls back to Director's documented default of 30.
        let (spec_scale, shininess) = if mat.shininess > 0.0 {
            (1.0, mat.shininess)
        } else {
            (mat.reflectivity.clamp(0.0, 1.0), 30.0)
        };
        gl.uniform4f(
            shader.u_specular_color.as_ref(),
            mat.specular[0] * spec_scale,
            mat.specular[1] * spec_scale,
            mat.specular[2] * spec_scale,
            mat.specular[3],
        );
        gl.uniform1f(shader.u_shininess.as_ref(), shininess);
        gl.uniform1f(shader.u_opacity.as_ref(), mat.opacity);
    }

    /// Set GL blend mode based on material opacity and shader blend function.
    /// `force_blend` = true when drawing in the transparent pass (models with alpha textures).
    fn apply_blend_mode(gl: &WebGl2RenderingContext, shader: &Shader3d, opacity: f32, first_layer_blend_func: u8, force_blend: bool) {
        // IFX first-layer blend func: 0 = IFX_SELECT_ARG0, 1 = IFX_ADD,
        // 2 = IFX_MODULATE (out = texture * lit color — the NORMAL lit case).
        // Blend func alone never disables lighting: a full-bright element (skybox,
        // galaxy backdrop) achieves that through an emissive material / bright ambient,
        // not by mislabelling IFX_MODULATE as "#replace". Treating blend_func==2 as
        // unlit flattened every ordinary textured model (e.g. the Dummy character,
        // whose whole face is IFX_MODULATE), so `u_texture_unlit` stays off here.
        //
        // Deriving "unlit" from SELECT_ARG0 (0) alone is not the answer either:
        // Burnin' Rubber authors its cars `#replace` and then has [PS] LightManager
        // set them back to `#multiply` at race start, precisely so they take the
        // dynamic light it paints. A movie that wants full-bright says so with
        // emissive / ambient.
        let _ = first_layer_blend_func;
        gl.uniform1i(shader.u_texture_unlit.as_ref(), 0);
        // Director's additive idiom sets `shader.blend = 0` so the BASE layer
        // contributes nothing, then adds the real contribution on an `#add`
        // layer. That 0 must not be folded into the additive draw as well or it
        // multiplies the result to nothing — the flash/sparks/streaks vanish.
        if first_layer_blend_func == 1 {
            gl.uniform1f(shader.u_opacity.as_ref(), 1.0);
        }
        if opacity < 1.0 || first_layer_blend_func == 1 || force_blend {
            gl.enable(WebGl2RenderingContext::BLEND);
            if first_layer_blend_func == 1 {
                // #add — additive blending (for glow/lightbox effects).
                // Alpha uses ONE/ONE so coverage only ever accumulates; see the
                // blend_func_separate calls above for why the alpha channel must
                // not share the colour factors.
                gl.blend_func_separate(
                    WebGl2RenderingContext::SRC_ALPHA,
                    WebGl2RenderingContext::ONE,
                    WebGl2RenderingContext::ONE,
                    WebGl2RenderingContext::ONE,
                );
            } else {
                // #multiply / default — standard alpha blending
                gl.blend_func_separate(
                    WebGl2RenderingContext::SRC_ALPHA,
                    WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
                    // Alpha must accumulate COVERAGE, not be blended with the same
                    // factors as colour. With blend_func the alpha channel gets
                    // dst.a = src.a*src.a + dst.a*(1-src.a), so a translucent draw over
                    // an opaque background LOWERS its alpha (0.3 over 1.0 -> 0.79).
                    // For a directToStage 3D sprite that layer is then composited over
                    // the 2D sprites, so every dust particle punched a hole through the
                    // scene and revealed the 2D sprites behind it — Heatwave Racing's
                    // loading text/bar (channels 51-53, live on the same frame as the
                    // race) bled through wherever dust was drawn.
                    WebGl2RenderingContext::ONE,
                    WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
                );
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
        resource_name: Symbol,
        model_name: Symbol,
        gpu_data: &MemberGpuData,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        // Only skin models that have a matching skeleton — no fallback to first()
        // to prevent walls/weapons from being skinned with the character skeleton.
        // Director is case-insensitive — script-side cloned resources can vary
        // case from the parsed W3D file.
        let skeleton = scene.skeletons.iter().find(|s| s.name == resource_name);
        let skeleton = match skeleton {
            Some(s) if s.bones.len() > 1 => s,
            _ => return false,
        };

        // The bind pose is the skeleton's REST pose, for every rig. IFX captures it once
        // at model-bind time (`CIFXSkeletonModifier::SetModelData` -> `StoreReferencePositions`)
        // by walking the loaded rest TRS, and stores it in a per-bone reference record
        // (`IFXCoreNodeShare+32`) that motion sampling never writes. A motion's frame 0 is
        // therefore never the bind pose — see docs/w3d-skeleton-motion-spec.md.
        let inv_bind_fresh = {
            let rest = crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, None, 0.0);
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
        let current_motion_name = bp.and_then(|b| b.current_motion)
            .or_else(|| runtime_state.and_then(|rs| rs.current_motion));
        let is_loop = bp.map(|b| b.animation_loop)
            .or_else(|| runtime_state.map(|rs| rs.animation_loop)).unwrap_or(true);
        let root_lock = bp.map(|b| b.root_lock)
            .or_else(|| runtime_state.map(|rs| rs.root_lock)).unwrap_or(false);
        let motion = if let Some(name) = current_motion_name {
            scene.motions.iter().find(|m| m.name == name)
        } else {
            // Nothing played yet. Director does NOT leave a skinned rig in its bind
            // pose here — it seeds the bonesPlayer with the rig's own motion at load,
            // so the model stands in that clip's frame 0 rather than the authored
            // T-pose. Agent Free Ride's boarder rode with his arms out because we
            // fell through to no motion at all.
            crate::director::chunks::w3d::skeleton::default_motion_for_model(scene, model_name)
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
        // Sample clock: a per-model bonesPlayer owns its own clock. Without one,
        // the ADVANCING legacy clock is only right when the legacy member fields
        // hold an explicitly played motion; a rig that never had play() called
        // stands in its seeded clip's FRAME 0 (Director pre-loads the playlist
        // but does not run it — the Agent Free Ride note on `current_motion_name`
        // above). Advancing the seeded clip animated Intel's DoNotPush "Barry"
        // rig from the moment its skeleton first parsed, drifting it off the
        // authored pose.
        let time = bp.map(|b| b.animation_time)
            .or_else(|| runtime_state.and_then(|rs| rs.current_motion.map(|_| self.animation_time)))
            .unwrap_or(0.0);
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
        } else {
            // Zero-length range = "hold exactly this frame", so hold it — do NOT
            // rewind to 0, which is frame 0 of the clip and, on a combined clip
            // authored from a T-pose, is literally the bind pose.
            //
            // Games freeze a pose this way: Rifleman ends every animation with
            // `queue(motion, 1, endTime, endTime, 0.0)` to hold the last frame,
            // and that entry LOOPS, so the soldier snapped to a T-pose and stayed
            // there until the next state change. Agent Free Ride 2's rider shows
            // the same flash on landing. `eff_start` is the frame the caller asked
            // for; when no range was ever set it is 0 anyway, so the old behaviour
            // is preserved for everything that was not asking for a hold.
            eff_start
        };
        // Root motion goes to the model NODE, not into the skin — see
        // `skeleton::motion_has_root_translation`. When the clock this draw uses
        // is a per-model bonesPlayer (the only case `tick_w3d_animations` pushes
        // clearance for) and the clip travels, strip the root translation here
        // and let the tick carry it on the node. The two are exactly
        // compensating, so the drawn mesh does not move.
        let strips_root = !root_lock
            && bp.is_some()
            && motion.map(|m| crate::director::chunks::w3d::skeleton::motion_has_root_translation(skeleton, m))
                .unwrap_or(false);
        let world_matrices = crate::director::chunks::w3d::skeleton::build_bone_matrices_ex(
            skeleton, motion, t, root_lock || strips_root,
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
        // (dino/frog) get no relativization at all.
        //
        // The idle MUST be one that drives THIS rig — `scene.motions` is a member-wide
        // table and a game can clone several skeletons plus all their clips into one
        // member. Keep this to an authored idle: it is a FALLBACK for models whose fold
        // was not recorded, and widening it relativizes draws that never were.
        let idle_root_mats = crate::director::chunks::w3d::skeleton::idle_reference_motion(scene, skeleton)
            .map(|im| crate::director::chunks::w3d::skeleton::build_bone_matrices(skeleton, Some(im), 0.0));
        // Only models with an idle-rest motion (the biped actors/bots) are relativized;
        // everything else (dino, frog01, ClubMarian, …) keeps the original skin — no
        // relativization — so this can't regress them.
        // The parser folds the biped COM into the model NODE at import, the way
        // Director does (see `apply_root_com_to_model_nodes`), and records the exact
        // matrix it used. Strip that same matrix here so the drawn mesh does not
        // move: (node * R0) * inv(R0) * world * inv_bind == node * world * inv_bind.
        // Taking R0 from the recorded value rather than recomputing it is what keeps
        // the two sides from drifting apart.
        let folded_com = scene.model_root_com.get(&model_name.to_ascii_lowercase())
            .or_else(|| scene.model_root_com.get(&resource_name.to_ascii_lowercase()));
        // A CLONE is deliberately absent from `model_root_com` (its fold can be
        // destroyed by a script assigning `transform`, and recording it there
        // blanked AreaZero's FPS weapon), but `clone_hop_count` carries the exact
        // r0 the hop applied. When the idle tier is about to fire for such a
        // model, strip THAT matrix instead of re-deriving one from a clip: the
        // whole point of recording r0 is that the fold and the strip must be the
        // same matrix, and for a clone they demonstrably are not.
        //
        // Street Sesh clones its skater out of "player_mike" — a member holding
        // the rig and no clips, so the parser folded its REST root, r0 =
        // (0, 4.42, -0.87) — and only afterwards clones `player_idle` & co into
        // the world member. `idle_reference_motion` then found `cpy_player2_idle`,
        // whose frame-0 root sits at the pelvis, (17.05, 22.48, 105.72), and
        // stripping that buried the skater to the waist in the plaza.
        //
        // Narrow ON PURPOSE: it only ever REPLACES the matrix of a strip that was
        // already going to happen. A model with no idle motion still gets no
        // strip, so this cannot introduce one where there was none.
        // …and ONLY while the node still holds it. A script that replaces the
        // node's matrix outright destroys the fold, and stripping it then
        // displaces the mesh instead of cancelling: AreaZero's
        // `[M] FPS Weapon.setup_Elite` hardcodes
        // `transform.rotation = vector(-90, 90, 0)` on its cloned "Elite" rig,
        // and stripping the carried r0 (whose root sits at the biped COM,
        // z = 104.5, against the idle clip's z = 2.8) threw the first-person
        // weapon out of frame entirely.
        let clone_r0 = runtime_state.and_then(|rs| {
            if rs.broken_root_com_fold.contains(&model_name) {
                return None;
            }
            rs.clone_hop_count.get(&model_name).map(|(_, r0)| *r0)
        });
        let root_relinv = match (folded_com, clone_r0, &idle_root_mats) {
            (Some(r0), _, _) => affine_inv(r0),
            (None, Some(r0), Some(m)) if !m.is_empty() => affine_inv(&r0),
            (None, _, Some(m)) if !m.is_empty() => affine_inv(&m[0]),
            // NO clone tier here. A 2026-08-18 attempt added a LAST tier that
            // relativized any cloneModelFromCastmember model by its posed root
            // (`world_matrices[0]`) to strip the biped COM from AreaZero's clip-less
            // "Punch" blade rig. It is refuted: `clone_hop_count` holds EVERY cloned
            // skinned model (hop >= 1), so the tier also fired on Agent Free Ride's
            // rider/vehicle rigs and on all six Rifleman `soldier_N` clones — probe:
            // `tier=clone_posed` for player/veh_player_1..5/soldier_1..6 — rotating
            // each by inv(root) (a 3ds-Max biped root is Rz(-90) here) and displacing
            // it by the root offset. That is the reported "AFR1/AFR2 player rotation"
            // and "Rifleman soldier aiming"; with the tier gone the AFR riders stand
            // on their boards again.
            //
            // It also contradicts a rule MEASURED in real Director 11.5 on Rifleman's
            // own spawn code (memory [[w3d-clone-hop-refold]]): the fold is re-applied
            // per clone HOP and a clone is deliberately never recorded in
            // `model_root_com`, because "the renderer's strip is only valid while the
            // node still holds the fold, and a script assigning `transform` destroys
            // it" — which is exactly what AreaZero's FPS weapon script does
            // (`transform.rotation = vector(-90,90,0)`). Any future fix for the blade
            // must key off the hop COUNT and the r0 the hop carried, not off "is a
            // clone at all".
            _ => IDENTITY_4X4,
        };

        // Check for motion blending (crossfade) — per-model blend state.
        let blend_weight = bp.map(|b| b.blend_weight).unwrap_or(self.blend_weight);
        let prev_motion_name = bp.and_then(|b| b.previous_motion.map(|s| s.as_str()))
            .or_else(|| runtime_state.and_then(|rs| rs.previous_motion.map(|s| s.as_str())));
        let blending = blend_weight < 1.0 && prev_motion_name.is_some();

        // 96 slots: Intel's own IFX sample rigs exceed the old 48-bone cap
        // (DoNotPush's "Barry_delib" skeleton is 82 bones) — with the cap, every
        // vertex weighted to a bone >= the cap rode a clamped wrong matrix and
        // the character drew as scrambled chunks. 96 mat4 = 384 vec4 uniforms,
        // well inside desktop WebGL2 vertex-uniform budgets.
        let bone_count = skeleton.bones.len().min(96);
        // Initialize ALL uniform slots to identity — bone indices can reference
        // any slot, even beyond the skeleton's actual bone count.
        let uniform_slots = 96;
        let mut skinning_matrices = vec![0.0f32; uniform_slots * 16];
        for i in 0..uniform_slots {
            skinning_matrices[i * 16]      = 1.0; // m[0][0]
            skinning_matrices[i * 16 + 5]  = 1.0; // m[1][1]
            skinning_matrices[i * 16 + 10] = 1.0; // m[2][2]
            skinning_matrices[i * 16 + 15] = 1.0; // m[3][3]
        }

        if blending {
            let prev_motion = prev_motion_name.and_then(|n| scene.motions.iter().find(|m| m.name == n));
            let prev_matrices = crate::director::chunks::w3d::skeleton::build_bone_matrices_ex(
                skeleton, prev_motion, t, root_lock || strips_root,
                if bone_overrides.is_empty() { None } else { Some(&bone_overrides) },
            );
            for i in 0..bone_count {
                let cur_rel = mat4_multiply_col_major(&root_relinv, &world_matrices[i]);
                let prev_rel = mat4_multiply_col_major(&root_relinv, &prev_matrices[i]);
                // Blend the two posed bone transforms with proper rotation SLERP (not an
                // element-wise matrix lerp, which collapses limbs mid-blend), then apply
                // the shared inverse-bind after the blend.
                let blended_rel = blend_bone_transform_trs(&prev_rel, &cur_rel, blend_weight);
                let final_mat = mat4_multiply_col_major(&blended_rel, &inv_bind[i]);
                skinning_matrices[i * 16..i * 16 + 16].copy_from_slice(&final_mat);
            }
        } else {
            for i in 0..bone_count {
                let rel = mat4_multiply_col_major(&root_relinv, &world_matrices[i]);
                let final_mat = mat4_multiply_col_major(&rel, &inv_bind[i]);
                skinning_matrices[i * 16..i * 16 + 16].copy_from_slice(&final_mat);
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
    fn is_child_of(&self, scene: &W3dScene, node_name: Symbol, root_name: Symbol) -> bool {
        if node_name == root_name { return true; }
        let mut current = node_name;
        for _ in 0..20 { // max depth to prevent infinite loops
            if let Some(node) = scene.nodes.iter().find(|n| n.name == current) {
                if node.parent_name == root_name { return true; }
                if node.parent_name.is_empty() { return false; }
                current = node.parent_name;
            } else {
                return false;
            }
        }
        false
    }

    /// Check if any ancestor in the parent chain is in the detached set
    fn has_detached_ancestor(&self, scene: &W3dScene, parent_name: Symbol, detached: &std::collections::HashSet<Symbol>) -> bool {
        if parent_name.is_empty() || parent_name == BuiltInSymbol::World { return false; }
        if detached.contains(&parent_name) { return true; }
        // Walk up parent chain
        for _ in 0..10 {
            if let Some(node) = scene.nodes.iter().find(|n| n.name == parent_name) {
                if node.parent_name.is_empty() || node.parent_name == BuiltInSymbol::World { return false; }
                if detached.contains(&node.parent_name) { return true; }
                return self.has_detached_ancestor(scene, node.parent_name, detached);
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
        let default_cam = BuiltInSymbol::DefaultView;
        let cam_name = self.active_camera.unwrap_or(default_cam.into());

        // 2. Find the camera node (case-insensitive), fall back to first view node
        let view_node = scene.nodes.iter()
            .find(|n| n.node_type == W3dNodeType::View && n.name == cam_name)
            .or_else(|| scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View));

        if let Some(node) = view_node {
            let world_t = self.accumulate_transform_with_state(scene, node, runtime_state);
            let cam_pos = [world_t[12], world_t[13], world_t[14]];
            return (invert_transform(&world_t), cam_pos);
        }
        let cam_name = view_node.map(|n| n.name).unwrap_or(BuiltInSymbol::DefaultView.into());

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
        let cam_name = self.active_camera.unwrap_or_else(|| Symbol::from_str("DefaultView"));
        // Find camera node (case-insensitive), fall back to first view node
        let view_node = scene.nodes.iter()
            .find(|n| n.node_type == W3dNodeType::View && n.name == cam_name)
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
            .and_then(|rs| rs.camera_projection_mode.get(&cam_name))
            .map(|&m| m == 1)
            .unwrap_or(false);

        let stored_ortho_h = runtime_state
            .and_then(|rs| rs.camera_ortho_height.get(&cam_name))
            .copied();

        let mut proj = if is_ortho {
            // Director's documented default orthoHeight is 200.0 world units.
            let ortho_h = stored_ortho_h.unwrap_or(200.0);
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
        let mut spot_exps = [0.0f32; 8];     // GL_SPOT_EXPONENT (0 = SpotDecay off, uniform)
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
        // Match on the CASE-NORMALISED spelling with lowercase literals.
        // `Symbol::as_str()` returns whichever casing was interned FIRST, so
        // `matches!(name, "UIAmbient")` silently stopped matching once anything
        // interned another casing. When `is_fallback_directional` missed, the
        // synthetic fallback key light was no longer suppressed for a movie that
        // lights itself and washed the scene; when `is_fallback_light` missed,
        // `has_movie_light` counted a fallback as a real movie light.
        let is_fallback_light = |name: &str| matches!(name,
            "defaultambient" | "defaultdirectional" | "uiambient" | "uidirectional");
        // `uidirectional` is NOT suppressed. The parser injects it exactly where
        // Director does — the member's parse-time default camera rotated -45 deg
        // about world X (measured on two movies, see parser.rs) — and Director
        // KEEPS it even once the movie adds directionals of its own: its
        // ChickenChasin light list is UIAmbient, UIDirectional, omni01, omni02
        // and three "default max light"s, all live. Suppressing it there dropped
        // the only light with a meaningful +Y and left the runtime-generated
        // terrain lit by ambient alone. `defaultdirectional` is a different
        // thing — dirplayer's own empty-scene invention, which Director has no
        // equivalent of — so that one is still suppressed when the movie lights
        // itself.
        let is_fallback_directional = |name: &str| matches!(name,
            "defaultdirectional");
        let has_movie_light = scene.lights.iter().any(|l|
            l.enabled && !is_fallback_light(l.name.as_lower_str())
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
                if has_movie_light && is_fallback_directional(light.name.as_lower_str()) {
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
                        if let Some(root) = rs.camera_root_nodes.get(&cam) {
                            if !self.is_child_of(scene, light.name, *root) {
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
                // SpotDecay ON → GL_SPOT_EXPONENT so the beam reaches 0.04 at the outer
                // cone edge (exp = log10(.04)/log10(cos(outer))); OFF → 0 (uniform + hard
                // cutoff). Clamp the log to avoid a blow-up for near-0° cones.
                spot_exps[li] = if lt == 3 && light.spot_decay {
                    let cone_cos = spot_angles[li].cos().min(0.9999).max(1e-4);
                    (0.04f32.ln() / cone_cos.ln()).clamp(0.0, 128.0)
                } else {
                    0.0
                };

                if let Some(light_node) = scene.nodes.iter().find(|n| {
                    n.node_type == W3dNodeType::Light && (n.resource_name == light.name || n.name == light.name)
                }) {
                    let world_t = self.accumulate_transform_with_state(scene, light_node, runtime_state);
                    if lt == 1 {
                        // Directional: the beam travels along the light's -Z (confirmed by
                        // the spot cone, which uses -Z as its aim). The shader's L is the
                        // direction *to* the light, i.e. the opposite = +Z. (The old code
                        // used -Z here, which only looked right because diffuse used
                        // abs(N·L); with one-sided max() the sign must be correct.)
                        positions[li * 3]     = world_t[8];
                        positions[li * 3 + 1] = world_t[9];
                        positions[li * 3 + 2] = world_t[10];
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
        gl.uniform1fv_with_f32_array(shader.u_light_spot_exp.as_ref(), &spot_exps[..n]);
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
/// Fraction of a texture's texels that must carry intermediate alpha before the
/// texture counts as translucent rather than an alpha-keyed cutout mask. An
/// anti-aliased cutout only spends its outline on partial alpha — a few percent
/// even for fine foliage — so the gap between the two populations is wide.
const SOFT_ALPHA_FRACTION: f32 = 0.20;

/// Second, independent translucency test, for a texture that is mostly EMPTY.
/// `SOFT_ALPHA_FRACTION` is measured over EVERY texel, so a small effect on a
/// large transparent field can never reach it however faint the effect is.
/// Judge those by the texels that are visible at all: an alpha-keyed cutout
/// keeps a solid alpha-255 interior and spends only its OUTLINE on partial
/// alpha — a perimeter-to-area ratio that stays well under a third even for fine
/// detail — while anything with a real ramp in it runs far above that.
///
/// The bar sits at a third rather than the 0.80 it started at because a mixed
/// ATLAS lands between the two populations: alpha-keyed sprites and genuinely
/// translucent art on one sheet, so the ramps are a minority of the visible
/// texels and a much smaller minority of the sheet. Burnin' Rubber 3's HUD atlas
/// `Interface_Texture` is measured at 1024x512 = 333230 clear, 76405 partial,
/// 114653 opaque: 14.6% of the sheet and 40.0% of its visible texels, which
/// missed both this test at 0.80 and `SOFT_ALPHA_FRACTION` at 0.20. Alpha-testing
/// it binarised the scoreboard plates — black art with an alpha ramp — into hard
/// black bars where the capture blends a gradient over the sky behind them.
///
/// Lowering this is strictly additive in the same way the test itself is: it can
/// only move a texture from the alpha-tested pass to the blended one, which is
/// the direction Director is always in.
const TRANSLUCENT_OF_VISIBLE_FRACTION: f32 = 0.33;

/// Floor on the partial-alpha texel COUNT for the test above, so a handful of
/// stray anti-aliased texels in an otherwise binary mask cannot carry it.
const TRANSLUCENT_MIN_SOFT_TEXELS: usize = 64;

/// Classify a decoded RGBA buffer as `(has_alpha, soft_alpha)`.
///
/// * `has_alpha` — the texture carries alpha at all, so it must not be drawn as
///   flat opaque geometry.
/// * `soft_alpha` — the alpha is a genuine translucency RAMP rather than an
///   alpha-keyed CUTOUT mask (foliage, decals, icon atlases, where only the
///   anti-aliased outline sits between fully-on and fully-off). Director always
///   alpha-blends; the alpha-tested cutout pass is our approximation and is only
///   equivalent for a binary mask. Applied to a ramp it quantises every texel to
///   fully-on/fully-off — AreaZero's MenuScanLines camera filter (55% mid-alpha)
///   came out as solid black bars.
///
/// Two independent tests, because one measure cannot cover both shapes:
///
/// 1. Mid-alpha over the WHOLE texture (`SOFT_ALPHA_FRACTION`). Catches a filter
///    or a haze that covers most of its own image.
/// 2. Mid-alpha over just the VISIBLE texels (`TRANSLUCENT_OF_VISIBLE_FRACTION`).
///    Catches a small, faint effect on a large empty field, which test 1 can
///    never reach however translucent it is. Rasterwerks' pulse-gun muzzle flash
///    (`Flarel~6`) is 77% fully transparent and only 6.7% mid-alpha overall, so
///    it was alpha-tested — which discards the faint 95% of the flash and draws
///    the rest as a hard, solid-white bar. Of the texels that are visible at
///    all, 97% are partial alpha: it is translucent, not a mask.
fn classify_texture_alpha(rgba_data: &[u8]) -> (bool, bool) {
    let has_alpha = rgba_data.chunks(4).any(|p| p[3] < 250);

    let total = rgba_data.len() / 4;
    let soft = rgba_data.chunks(4).filter(|p| p[3] >= 16 && p[3] < 240).count();
    let opaque = rgba_data.chunks(4).filter(|p| p[3] >= 240).count();
    let visible = soft + opaque;
    let mostly_translucent = soft >= TRANSLUCENT_MIN_SOFT_TEXELS
        && visible > 0
        && (soft as f32 / visible as f32) > TRANSLUCENT_OF_VISIBLE_FRACTION;
    let soft_alpha = total > 0
        && ((soft as f32 / total as f32) > SOFT_ALPHA_FRACTION || mostly_translucent);

    (has_alpha, soft_alpha)
}

fn decode_and_upload_texture_impl(context: &WebGL2Context, data: &[u8], flip_v: bool, near_filtering: bool, quality: Option<&str>) -> Option<(WebGlTexture, u32, u32, bool, bool)> {
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
                            // The alpha block is stored BOTTOM-UP relative to the
                            // JPEG's top-down rows, so it has to be flipped before
                            // it's married to the colour. Applied row-for-row it
                            // lines the mask up with the mirror image of the
                            // artwork: Heatwave Racing's palm trees kept the
                            // sky-blue background between their fronds and cut
                            // holes out of the foliage instead.
                            let (wu, hu) = (w as usize, h as usize);
                            for y in 0..hu {
                                let src = (hu - 1 - y) * wu;
                                for x in 0..wu {
                                    rgba[(y * wu + x) * 4 + 3] = alpha[src + x];
                                }
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

    // `nearFiltering` FALSE means the movie asked for NO bilinear filtering on
    // this texture (Director 11.5 Scripting Dictionary; default TRUE). Movies
    // bake UI text into textures and turn it off precisely so the glyphs stay
    // pixel-crisp — smoothing them spreads a one-pixel stem over two pixels at
    // half intensity, which is exactly what made AreaZero's in-game controls
    // list unreadable where it crossed bright geometry. Minification stays
    // mipmapped either way, so distant geometry does not start aliasing.
    //
    // `quality` (same dictionary) chooses the MIPMAPPING level on top of that:
    // `#low` none, `#medium` bilinear, `#high` trilinear. Its documented default
    // is `#low`, but this engine has always mipmapped everything and a movie
    // that never mentions `quality` keeps that — a DELIBERATE divergence, since
    // switching every untouched texture to unmipmapped would make distant
    // geometry alias across every movie at once. A movie that DOES set it gets
    // what it asked for. The undocumented `#lowFiltered` family is treated as
    // its base level.
    let mip = match quality.map(|q| q.to_ascii_lowercase()) {
        Some(ref q) if q.starts_with("low") => Some(false),
        Some(ref q) if q.starts_with("medium") || q.starts_with("high") => Some(true),
        _ => None,
    };
    let (min_filter, mag_filter) = match (near_filtering, mip) {
        (true, Some(false)) => (WebGl2RenderingContext::LINEAR, WebGl2RenderingContext::LINEAR),
        (true, _) => (WebGl2RenderingContext::LINEAR_MIPMAP_LINEAR, WebGl2RenderingContext::LINEAR),
        (false, Some(false)) => (WebGl2RenderingContext::NEAREST, WebGl2RenderingContext::NEAREST),
        (false, _) => (WebGl2RenderingContext::NEAREST_MIPMAP_NEAREST, WebGl2RenderingContext::NEAREST),
    };
    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MIN_FILTER, min_filter as i32);
    gl.tex_parameteri(WebGl2RenderingContext::TEXTURE_2D, WebGl2RenderingContext::TEXTURE_MAG_FILTER, mag_filter as i32);
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
    let (has_alpha, soft_alpha) = classify_texture_alpha(&rgba_data);

    Some((texture, width, height, has_alpha, soft_alpha))
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
/// Pack per-vertex bone influences into fixed vec4 index + weight arrays. IFX keeps up
/// to 6 influences per vertex; the GPU path caps at 4. The stream writes them sorted by
/// descending weight with bone[0] carrying the residual `1-Σothers`, but nothing in the
/// format guarantees that, so a naive take(4) could drop the HEAVIEST bones and pull a
/// >4-influence vertex (spine/shoulder/hip) toward the wrong joints. Sort each vertex's
/// (index, weight) pairs by weight descending, keep the 4 largest, then renormalize the
/// survivors to sum 1.
fn pack_bone_influences_sorted(indices: &[Vec<u32>], weights: &[Vec<f32>]) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let mut idx_out = Vec::with_capacity(indices.len());
    let mut wgt_out = Vec::with_capacity(indices.len());
    for (vi, idxs) in indices.iter().enumerate() {
        let wts = weights.get(vi).map(|w| w.as_slice()).unwrap_or(&[]);
        let mut pairs: Vec<(u32, f32)> = idxs.iter().enumerate()
            .map(|(k, &b)| (b, wts.get(k).copied().unwrap_or(0.0).max(0.0)))
            .collect();
        pairs.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        pairs.truncate(4);
        let mut idx4 = [0.0f32; 4];
        let mut wgt4 = [0.0f32; 4];
        for (k, &(b, w)) in pairs.iter().enumerate() {
            idx4[k] = (b as f32).min(95.0); // clamp to bone uniform array size
            wgt4[k] = w;
        }
        let sum: f32 = wgt4.iter().sum();
        if sum > 0.001 { for w in wgt4.iter_mut() { *w /= sum; } } else { wgt4[0] = 1.0; }
        idx_out.push(idx4);
        wgt_out.push(wgt4);
    }
    (idx_out, wgt_out)
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
    kf.to_column_major_matrix()
}

/// Case-insensitive lookup in node_transforms (Director is case-insensitive for node names).
fn get_runtime_transform(rs: &crate::player::cast_member::Shockwave3dRuntimeState, name: Symbol) -> Option<[f32; 16]> {
    // No linear fallback. `Symbol` derives Hash/Eq over one interned `Spur`, so
    // the hash lookup and `==` are the SAME comparison — a follow-up scan could
    // never find anything `get` missed. (Symbol identity is already
    // case-insensitive: `intern` lowercases.) The scan only ever ran to
    // completion on a MISS, which is the common case since most nodes carry no
    // runtime override, making every miss O(node_transforms).
    //
    // That is quadratic in the wrong place: this is called per node per parent-
    // chain hop per frame, while `node_transforms` grows with everything the
    // movie spawns. In an AreaZero profile at higher waves it was the single
    // hottest pair in the whole frame — `get_runtime_transform` 14.7% self and
    // `hashbrown::map::Iter::next` 13.3% self. `raycast.rs` had the same bug and
    // the same fix; this copy was missed.
    rs.node_transforms.get(&name).copied()
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
    // Column-major: element (row, col) = m[col * 4 + row].
    //
    // This must be a GENERAL affine inverse, not the transpose. Transposing only
    // inverts an orthonormal rotation; for a scaled basis R = s·Q the true inverse
    // is Rᵀ/s², so a transpose is off by s². Director scripts routinely scale a
    // camera's transform — AreaZero's [PS] Camera does
    //     t.camera.transform.scale = vector(1, 1, 1) * t.scale   -- 0.1
    // every frame. At s = 0.1 the view matrix came out 100× too small, collapsing
    // the scene to a fraction of a unit in front of the camera, where hither = 1.0
    // clipped it: the robot was sliced and the panorama vanished. The projection
    // itself is scale-invariant (x, y and z scale together), so getting the inverse
    // right restores the framing without changing the apparent image.
    let (m00, m01, m02) = (m[0], m[4], m[8]);
    let (m10, m11, m12) = (m[1], m[5], m[9]);
    let (m20, m21, m22) = (m[2], m[6], m[10]);
    let tx = m[12]; let ty = m[13]; let tz = m[14];

    let c00 = m11 * m22 - m12 * m21;
    let c01 = m12 * m20 - m10 * m22;
    let c02 = m10 * m21 - m11 * m20;
    let det = m00 * c00 + m01 * c01 + m02 * c02;

    if det.abs() < 1e-12 {
        // Degenerate basis (zero scale) — fall back to the transpose so a broken
        // transform yields something finite rather than NaNs.
        let itx = -(m00 * tx + m10 * ty + m20 * tz);
        let ity = -(m01 * tx + m11 * ty + m21 * tz);
        let itz = -(m02 * tx + m12 * ty + m22 * tz);
        return [
            m00, m01, m02, 0.0,
            m10, m11, m12, 0.0,
            m20, m21, m22, 0.0,
            itx, ity, itz, 1.0,
        ];
    }

    let inv_det = 1.0 / det;
    // inv[row][col]
    let i00 = c00 * inv_det;
    let i01 = (m02 * m21 - m01 * m22) * inv_det;
    let i02 = (m01 * m12 - m02 * m11) * inv_det;
    let i10 = c01 * inv_det;
    let i11 = (m00 * m22 - m02 * m20) * inv_det;
    let i12 = (m02 * m10 - m00 * m12) * inv_det;
    let i20 = c02 * inv_det;
    let i21 = (m01 * m20 - m00 * m21) * inv_det;
    let i22 = (m00 * m11 - m01 * m10) * inv_det;

    // translation = -M⁻¹ · t
    let itx = -(i00 * tx + i01 * ty + i02 * tz);
    let ity = -(i10 * tx + i11 * ty + i12 * tz);
    let itz = -(i20 * tx + i21 * ty + i22 * tz);

    [
        i00, i10, i20, 0.0, // column 0
        i01, i11, i21, 0.0, // column 1
        i02, i12, i22, 0.0, // column 2
        itx, ity, itz, 1.0,
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
/// Blend two column-major affine bone transforms by decomposing each into
/// translation + rotation(quaternion) + scale, LERPing translation/scale and
/// SLERPing rotation, then recomposing. A motion crossfade must interpolate real
/// rotations — an element-wise matrix lerp of two rotation bases is non-orthonormal
/// and collapses/skews limbs at mid-blend. Mirrors IFX IFXCharacter::BlendBoneNode
/// (blend the bone transform, then build the matrix).
fn blend_bone_transform_trs(a: &[f32; 16], b: &[f32; 16], t: f32) -> [f32; 16] {
    fn decomp(m: &[f32; 16]) -> ([f32; 3], [f32; 4], [f32; 3]) {
        let sx = (m[0] * m[0] + m[1] * m[1] + m[2] * m[2]).sqrt();
        let sy = (m[4] * m[4] + m[5] * m[5] + m[6] * m[6]).sqrt();
        let sz = (m[8] * m[8] + m[9] * m[9] + m[10] * m[10]).sqrt();
        let (ix, iy, iz) = (1.0 / sx.max(1e-8), 1.0 / sy.max(1e-8), 1.0 / sz.max(1e-8));
        // normalized rotation columns → r_{row,col}
        let (r00, r10, r20) = (m[0] * ix, m[1] * ix, m[2] * ix);
        let (r01, r11, r21) = (m[4] * iy, m[5] * iy, m[6] * iy);
        let (r02, r12, r22) = (m[8] * iz, m[9] * iz, m[10] * iz);
        let tr = r00 + r11 + r22;
        let q = if tr > 0.0 {
            let s = 0.5 / (tr + 1.0).sqrt();
            [(r21 - r12) * s, (r02 - r20) * s, (r10 - r01) * s, 0.25 / s]
        } else if r00 > r11 && r00 > r22 {
            let s = 2.0 * (1.0 + r00 - r11 - r22).sqrt();
            [0.25 * s, (r01 + r10) / s, (r02 + r20) / s, (r21 - r12) / s]
        } else if r11 > r22 {
            let s = 2.0 * (1.0 + r11 - r00 - r22).sqrt();
            [(r01 + r10) / s, 0.25 * s, (r12 + r21) / s, (r02 - r20) / s]
        } else {
            let s = 2.0 * (1.0 + r22 - r00 - r11).sqrt();
            [(r02 + r20) / s, (r12 + r21) / s, 0.25 * s, (r10 - r01) / s]
        };
        ([m[12], m[13], m[14]], q, [sx, sy, sz])
    }
    let (ta, qa, sa) = decomp(a);
    let (tb, qb, sb) = decomp(b);
    // SLERP qa → qb (shortest arc)
    let mut qb2 = qb;
    let mut dot = qa[0] * qb[0] + qa[1] * qb[1] + qa[2] * qb[2] + qa[3] * qb[3];
    if dot < 0.0 { for k in 0..4 { qb2[k] = -qb2[k]; } dot = -dot; }
    let q = if dot > 0.9995 {
        let mut r = [0.0f32; 4];
        for k in 0..4 { r[k] = qa[k] + (qb2[k] - qa[k]) * t; }
        let n = (r[0] * r[0] + r[1] * r[1] + r[2] * r[2] + r[3] * r[3]).sqrt().max(1e-8);
        [r[0] / n, r[1] / n, r[2] / n, r[3] / n]
    } else {
        let theta = dot.clamp(-1.0, 1.0).acos();
        let st = theta.sin();
        let (wa, wb) = (((1.0 - t) * theta).sin() / st, (t * theta).sin() / st);
        [qa[0] * wa + qb2[0] * wb, qa[1] * wa + qb2[1] * wb, qa[2] * wa + qb2[2] * wb, qa[3] * wa + qb2[3] * wb]
    };
    let tr = [ta[0] + (tb[0] - ta[0]) * t, ta[1] + (tb[1] - ta[1]) * t, ta[2] + (tb[2] - ta[2]) * t];
    let sc = [sa[0] + (sb[0] - sa[0]) * t, sa[1] + (sb[1] - sa[1]) * t, sa[2] + (sb[2] - sa[2]) * t];
    let (x, y, z, w) = (q[0], q[1], q[2], q[3]);
    let (xx, yy, zz) = (x * x, y * y, z * z);
    let (xy, xz, yz) = (x * y, x * z, y * z);
    let (wx, wy, wz) = (w * x, w * y, w * z);
    [
        (1.0 - 2.0 * (yy + zz)) * sc[0], (2.0 * (xy + wz)) * sc[0], (2.0 * (xz - wy)) * sc[0], 0.0,
        (2.0 * (xy - wz)) * sc[1], (1.0 - 2.0 * (xx + zz)) * sc[1], (2.0 * (yz + wx)) * sc[1], 0.0,
        (2.0 * (xz + wy)) * sc[2], (2.0 * (yz - wx)) * sc[2], (1.0 - 2.0 * (xx + yy)) * sc[2], 0.0,
        tr[0], tr[1], tr[2], 1.0,
    ]
}

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

#[cfg(test)]
mod alpha_classification_tests {
    use super::classify_texture_alpha;

    /// Build a 128x128 RGBA buffer from a per-texel alpha function.
    fn tex(alpha_at: impl Fn(usize, usize) -> u8) -> Vec<u8> {
        let mut out = Vec::with_capacity(128 * 128 * 4);
        for y in 0..128 {
            for x in 0..128 {
                out.extend_from_slice(&[255, 255, 255, alpha_at(x, y)]);
            }
        }
        out
    }

    #[test]
    fn opaque_texture_has_no_alpha() {
        let (has_alpha, soft) = classify_texture_alpha(&tex(|_, _| 255));
        assert!(!has_alpha);
        assert!(!soft);
    }

    /// A HUD atlas: alpha-keyed sprites sharing the sheet with genuinely
    /// translucent art. Neither of the first two tests can see it — the ramps are
    /// a minority of the sheet, and the keyed sprites keep solid interiors — but
    /// alpha-testing it binarises the ramps.
    ///
    /// Proportioned from the measured Burnin' Rubber 3 `Interface_Texture`
    /// (1024x512: 63.6% clear, 14.6% partial, 21.9% opaque — 40.0% of the visible
    /// texels partial), which drew the scoreboard plates as hard black bars.
    #[test]
    fn mixed_atlas_with_translucent_art_is_soft() {
        let atlas = tex(|_x, y| {
            if y < 81 { 0 }                 // 63.3% clear
            else if y < 100 { 96 }          // 14.8% partial-alpha ramp art
            else { 255 }                    // 21.9% alpha-keyed sprite interiors
        });
        let (has_alpha, soft) = classify_texture_alpha(&atlas);
        assert!(has_alpha);
        assert!(
            soft,
            "an atlas whose visible texels are 40% partial alpha is not a binary mask"
        );
    }

    /// ...but the same shape at a cutout's perimeter-to-area ratio must NOT trip
    /// it, or every anti-aliased sprite sheet would move to the blended pass and
    /// stop writing depth. Same clear/visible split, a quarter of the mid-alpha.
    #[test]
    fn atlas_of_antialiased_cutouts_is_not_soft() {
        let atlas = tex(|_x, y| {
            if y < 81 { 0 }
            else if y < 86 { 96 }           // ~10% of visible: an AA outline
            else { 255 }
        });
        let (has_alpha, soft) = classify_texture_alpha(&atlas);
        assert!(has_alpha);
        assert!(!soft, "an anti-aliased sprite sheet is still a cutout mask");
    }

    /// A binary cutout — a solid alpha-255 disc with a one-texel anti-aliased
    /// rim — must stay in the alpha-TESTED pass so it keeps writing depth.
    #[test]
    fn binary_cutout_is_not_soft() {
        let mask = tex(|x, y| {
            let d = (((x as f32) - 64.0).powi(2) + ((y as f32) - 64.0).powi(2)).sqrt();
            if d < 40.0 { 255 } else if d < 41.0 { 128 } else { 0 }
        });
        let (has_alpha, soft) = classify_texture_alpha(&mask);
        assert!(has_alpha);
        assert!(!soft, "an anti-aliased disc is a cutout mask, not a translucency ramp");
    }

    /// A faint effect on a large empty field — the shape of every muzzle flash,
    /// spark and blood decal. Its mid-alpha texels are a small share of the
    /// TEXTURE but almost all of what is visible, so it must be BLENDED.
    /// Alpha-testing it at 0.5 is what drew Rasterwerks' pulse-gun flash as a
    /// solid white bar.
    #[test]
    fn faint_flare_on_empty_field_is_soft() {
        // A horizontal streak across the middle 8 rows, alpha 8..64 — well under
        // the 0.5 alpha test, and only 6% of the texture.
        let flare = tex(|x, _y2| (8 + (x % 56)) as u8);
        let flare = {
            let mut v = flare;
            for y in 0..128 {
                for x in 0..128 {
                    if !(60..68).contains(&y) {
                        v[(y * 128 + x) * 4 + 3] = 0;
                    }
                }
            }
            v
        };
        let (has_alpha, soft) = classify_texture_alpha(&flare);
        assert!(has_alpha);
        assert!(soft, "a faint streak on a transparent field is translucent, not a mask");
    }

    /// The floor guards the visible-texel test against noise: a handful of
    /// stray anti-aliased texels must not make an otherwise binary mask soft.
    #[test]
    fn a_few_stray_soft_texels_do_not_make_a_mask_soft() {
        let mut data = tex(|_, _| 0);
        for i in 0..16 {
            data[i * 4 + 3] = 100;
        }
        let (_has_alpha, soft) = classify_texture_alpha(&data);
        assert!(!soft);
    }
}
