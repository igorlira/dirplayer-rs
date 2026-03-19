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

use super::context::WebGL2Context;
use super::mesh3d::Mesh3dBuffers;
use crate::director::chunks::w3d::types::*;

/// GPU state for a single Shockwave3D member
struct MemberGpuData {
    /// Mesh buffers keyed by resource name (matches ModelNode.model_resource_name)
    mesh_groups: HashMap<String, Vec<Mesh3dBuffers>>,
    /// All meshes in upload order (fallback when no scene graph match)
    all_meshes: Vec<Mesh3dBuffers>,
    /// Texture images decoded and uploaded to GPU
    textures: HashMap<String, WebGlTexture>,
    /// Snapshot of scene content counts when GPU data was built
    scene_version: (usize, usize, usize, usize), // (nodes, clod_meshes, texture_images, shaders)
}

/// 3D shader program with uniform locations
struct Shader3d {
    program: WebGlProgram,
    u_model: Option<WebGlUniformLocation>,
    u_view: Option<WebGlUniformLocation>,
    u_projection: Option<WebGlUniformLocation>,
    u_diffuse_color: Option<WebGlUniformLocation>,
    u_ambient_color: Option<WebGlUniformLocation>,
    u_specular_color: Option<WebGlUniformLocation>,
    u_emissive_color: Option<WebGlUniformLocation>,
    u_shininess: Option<WebGlUniformLocation>,
    u_opacity: Option<WebGlUniformLocation>,
    u_diffuse_tex: Option<WebGlUniformLocation>,
    u_has_texture: Option<WebGlUniformLocation>,
    u_lightmap_tex: Option<WebGlUniformLocation>,
    u_has_lightmap: Option<WebGlUniformLocation>,
    u_lightmap_intensity: Option<WebGlUniformLocation>,
    u_has_texcoord2: Option<WebGlUniformLocation>,
    u_num_lights: Option<WebGlUniformLocation>,
    u_light_pos: Option<WebGlUniformLocation>,
    u_light_color: Option<WebGlUniformLocation>,
    u_light_type: Option<WebGlUniformLocation>,
    u_camera_pos: Option<WebGlUniformLocation>,
    u_global_ambient: Option<WebGlUniformLocation>,
    u_fog_enabled: Option<WebGlUniformLocation>,
    u_fog_near: Option<WebGlUniformLocation>,
    u_fog_far: Option<WebGlUniformLocation>,
    u_fog_color: Option<WebGlUniformLocation>,
    u_fog_mode: Option<WebGlUniformLocation>,
}

/// Particle billboard shader
struct ParticleShader {
    program: WebGlProgram,
    u_view_projection: Option<WebGlUniformLocation>,
    u_camera_right: Option<WebGlUniformLocation>,
    u_camera_up: Option<WebGlUniformLocation>,
    u_color_start: Option<WebGlUniformLocation>,
    u_color_end: Option<WebGlUniformLocation>,
    u_size: Option<WebGlUniformLocation>,
    u_lifetime: Option<WebGlUniformLocation>,
}

/// Manages 3D rendering for all Shockwave3D members
pub struct Scene3dRenderer {
    shader: Option<Shader3d>,
    particle_shader: Option<ParticleShader>,
    member_data: HashMap<(i32, i32), MemberGpuData>,
    pub fbo: Option<WebGlFramebuffer>,
    pub fbo_texture: Option<WebGlTexture>,
    fbo_depth: Option<web_sys::WebGlRenderbuffer>,
    fbo_width: u32,
    fbo_height: u32,
    logged_members: std::collections::HashSet<(i32, i32)>,
    animation_time: f32,
    motion_transforms: HashMap<String, [f32; 16]>,
    pub active_camera: Option<String>,
}

impl Scene3dRenderer {
    pub fn new() -> Self {
        Self {
            shader: None,
            particle_shader: None,
            member_data: HashMap::new(),
            fbo: None,
            fbo_texture: None,
            fbo_depth: None,
            fbo_width: 0,
            fbo_height: 0,
            logged_members: std::collections::HashSet::new(),
            animation_time: 0.0,
            motion_transforms: HashMap::new(),
            active_camera: None,
        }
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

uniform mat4 u_model;
uniform mat4 u_view;
uniform mat4 u_projection;

out vec3 v_position;
out vec3 v_normal;
out vec2 v_texcoord;
out vec2 v_texcoord2;
out float v_view_dist;

void main() {
    vec4 world_pos = u_model * vec4(a_position, 1.0);
    vec4 view_pos = u_view * world_pos;
    v_position = world_pos.xyz;
    v_normal = mat3(u_model) * a_normal;
    // W3D CLOD UVs are in [-0.5, 0.5] range — remap to [0, 1]
    // IFX applies V-flip via texture matrix: new_v = 1 - v
    v_texcoord = vec2(a_texcoord.x + 0.5, 0.5 - a_texcoord.y);
    v_texcoord2 = vec2(a_texcoord2.x + 0.5, 0.5 - a_texcoord2.y);
    v_view_dist = -view_pos.z;
    gl_Position = u_projection * view_pos;
}
"#;

        let fs_source = r#"#version 300 es
precision mediump float;

in vec3 v_position;
in vec3 v_normal;
in vec2 v_texcoord;
in vec2 v_texcoord2;
in float v_view_dist;

uniform vec4 u_diffuse_color;
uniform vec4 u_ambient_color;
uniform vec4 u_specular_color;
uniform vec4 u_emissive_color;
uniform float u_shininess;
uniform float u_opacity;
uniform sampler2D u_diffuse_tex;
uniform int u_has_texture;
uniform sampler2D u_lightmap_tex;
uniform int u_has_lightmap;
uniform float u_lightmap_intensity;
uniform int u_has_texcoord2;

uniform int u_num_lights;
uniform vec3 u_light_pos[8];
uniform vec3 u_light_color[8];
uniform int u_light_type[8];
uniform vec3 u_camera_pos;
uniform vec3 u_global_ambient;

// Fog
uniform int u_fog_enabled;
uniform float u_fog_near;
uniform float u_fog_far;
uniform vec3 u_fog_color;
uniform int u_fog_mode; // 0=linear, 1=exp, 2=exp2

out vec4 frag_color;

void main() {
    vec3 N = normalize(v_normal);
    vec3 V = normalize(u_camera_pos - v_position);

    vec4 tex_sample = texture(u_diffuse_tex, v_texcoord);

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
                    atten = 1.0 / (1.0 + 0.01 * dist + 0.0001 * dist * dist);
                }
                float diff = max(dot(N, L), 0.0);
                // Per-light: ambient fill (lightColor * matAmbient) + diffuse (NdotL * lightColor * WHITE)
                // WHITE because UseDiffuse=OFF forces materialDiffuse to (1,1,1)
                lighting += atten * (u_light_color[i] * u_ambient_color.rgb
                          + diff * u_light_color[i]);
            }
        }

        // IFX fixed-function clamps per-vertex lighting to [0,1] before GL_MODULATE
        lighting = clamp(lighting, vec3(0.0), vec3(1.0));
        // GL_MODULATE: fragment = texture * lighting
        vec3 final_color = tex_sample.rgb * lighting;

        // Apply second texture layer (shadow/lightmap) if present
        if (u_has_lightmap > 0) {
            // Use 2nd UV set if available, otherwise same as primary
            vec2 lm_uv = (u_has_texcoord2 > 0) ? v_texcoord2 : v_texcoord;
            vec4 lm_sample = texture(u_lightmap_tex, lm_uv);
            float intensity = u_lightmap_intensity;
            if (u_has_lightmap == 1) {
                // Multiply blend (shadow map): darken lit areas
                final_color *= mix(vec3(1.0), lm_sample.rgb, intensity);
            } else if (u_has_lightmap == 2) {
                // Additive blend (lightmap): brighten with light data
                final_color += lm_sample.rgb * intensity;
            }
        }

        frag_color = vec4(final_color, u_opacity * tex_sample.a);
        return;
    }

    vec3 base_color = u_diffuse_color.rgb;

    vec3 result = u_emissive_color.rgb + u_global_ambient * u_ambient_color.rgb;

    for (int i = 0; i < 8; i++) {
        if (i >= u_num_lights) break;
        if (u_light_type[i] == 0) {
            result += u_light_color[i] * u_ambient_color.rgb;
        } else {
            vec3 L;
            float atten = 1.0;
            if (u_light_type[i] == 1) {
                // Directional
                L = normalize(u_light_pos[i]);
            } else {
                // Point / Spot
                vec3 light_dir = u_light_pos[i] - v_position;
                float dist = length(light_dir);
                L = light_dir / dist;
                atten = 1.0 / (1.0 + 0.01 * dist + 0.0001 * dist * dist);
            }

            float diff = max(dot(N, L), 0.0);
            // Per-light ambient fill + diffuse (matches IFX per-light contribution)
            result += atten * (u_light_color[i] * u_ambient_color.rgb
                    + u_light_color[i] * base_color * diff);

            if (u_shininess > 0.0 && diff > 0.0) {
                vec3 H = normalize(L + V);
                float spec = pow(max(dot(N, H), 0.0), u_shininess);
                result += u_light_color[i] * u_specular_color.rgb * spec * atten;
            }
        }
    }

    // Apply fog
    if (u_fog_enabled > 0) {
        float fog_factor;
        if (u_fog_mode == 0) {
            // Linear
            fog_factor = clamp((u_fog_far - v_view_dist) / (u_fog_far - u_fog_near), 0.0, 1.0);
        } else if (u_fog_mode == 1) {
            // Exponential
            float density = 2.0 / (u_fog_far - u_fog_near);
            fog_factor = exp(-density * v_view_dist);
        } else {
            // Exponential squared
            float density = 2.0 / (u_fog_far - u_fog_near);
            fog_factor = exp(-density * density * v_view_dist * v_view_dist);
        }
        result = mix(u_fog_color, result, clamp(fog_factor, 0.0, 1.0));
    }

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
            u_ambient_color: u("u_ambient_color"),
            u_specular_color: u("u_specular_color"),
            u_emissive_color: u("u_emissive_color"),
            u_shininess: u("u_shininess"),
            u_opacity: u("u_opacity"),
            u_diffuse_tex: u("u_diffuse_tex"),
            u_has_texture: u("u_has_texture"),
            u_lightmap_tex: u("u_lightmap_tex"),
            u_has_lightmap: u("u_has_lightmap"),
            u_lightmap_intensity: u("u_lightmap_intensity"),
            u_has_texcoord2: u("u_has_texcoord2"),
            u_num_lights: u("u_num_lights"),
            u_light_pos: u("u_light_pos[0]"),
            u_light_color: u("u_light_color[0]"),
            u_light_type: u("u_light_type[0]"),
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
uniform float u_size;
uniform float u_lifetime;

out float v_age_ratio;
out vec2 v_uv;

void main() {
    v_age_ratio = clamp(a_age / u_lifetime, 0.0, 1.0);
    v_uv = a_corner * 0.5 + 0.5;

    // Size fades out near end of life
    float size_factor = u_size * (1.0 - v_age_ratio * 0.5);

    vec3 world_pos = a_center
        + u_camera_right * a_corner.x * size_factor
        + u_camera_up * a_corner.y * size_factor;

    gl_Position = u_view_projection * vec4(world_pos, 1.0);
}
"#;

        let fs = r#"#version 300 es
precision mediump float;

in float v_age_ratio;
in vec2 v_uv;

uniform vec3 u_color_start;
uniform vec3 u_color_end;

out vec4 frag_color;

void main() {
    // Circular soft particle
    float dist = length(v_uv - 0.5) * 2.0;
    if (dist > 1.0) discard;
    float alpha = 1.0 - dist * dist;

    // Fade out with age
    alpha *= 1.0 - v_age_ratio;

    vec3 color = mix(u_color_start, u_color_end, v_age_ratio);
    frag_color = vec4(color, alpha);
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
            u_size: u("u_size"),
            u_lifetime: u("u_lifetime"),
            program,
        });

        Ok(())
    }

    /// Render all active particle systems
    fn render_particles(
        &mut self,
        context: &WebGL2Context,
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
        let shader = self.particle_shader.as_ref().unwrap();

        gl.use_program(Some(&shader.program));

        // Compute view-projection matrix
        let vp = mat4_multiply_col_major(projection_matrix, view_matrix);
        gl.uniform_matrix4fv_with_f32_array(shader.u_view_projection.as_ref(), false, &vp);

        // Extract camera right/up from view matrix (inverse of view = camera world)
        // View matrix columns 0,1 in row-major = camera right, up in world space
        gl.uniform3f(shader.u_camera_right.as_ref(), view_matrix[0], view_matrix[4], view_matrix[8]);
        gl.uniform3f(shader.u_camera_up.as_ref(), view_matrix[1], view_matrix[5], view_matrix[9]);

        // Enable additive blending for particles
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.depth_mask(false); // Don't write to depth buffer

        for (_name, ps) in &rs.particles {
            if ps.positions.is_empty() { continue; }

            gl.uniform1f(shader.u_size.as_ref(), ps.particle_size);
            gl.uniform1f(shader.u_lifetime.as_ref(), ps.lifetime);
            gl.uniform3f(shader.u_color_start.as_ref(), 1.0, 1.0, 0.5); // yellow-ish
            gl.uniform3f(shader.u_color_end.as_ref(), 1.0, 0.2, 0.0);   // red-ish

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

        // Restore state
        gl.depth_mask(true);
        gl.enable(WebGl2RenderingContext::CULL_FACE);
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);

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
        let current_version = (scene.nodes.len(), scene.clod_meshes.len(), scene.texture_images.len(), scene.shaders.len());
        if let Some(existing) = self.member_data.get(&key) {
            if existing.scene_version == current_version {
                return Ok(());
            }
            // Scene changed — remove stale data and rebuild
            self.logged_members.remove(&key);
        }
        self.member_data.remove(&key);

        let mut mesh_groups: HashMap<String, Vec<Mesh3dBuffers>> = HashMap::new();
        let mut all_meshes = Vec::new();

        // Upload CLOD meshes (keyed by resource name)
        for (name, decoded_meshes) in &scene.clod_meshes {
            let mut group = Vec::new();
            for (mi, mesh) in decoded_meshes.iter().enumerate() {
                if mesh.positions.is_empty() || mesh.faces.is_empty() {
                    continue;
                }
                // Use decoded texcoords, or generate planar UVs if all texcoords are identical
                let tc_data;
                let tc = if !mesh.tex_coords.is_empty() && !mesh.tex_coords[0].is_empty() {
                    let tcs = &mesh.tex_coords[0];
                    // Check if all texcoords are identical (needs planar UV generation)
                    let all_same = tcs.len() > 1 && tcs.iter().all(|t| (t[0] - tcs[0][0]).abs() < 0.001 && (t[1] - tcs[0][1]).abs() < 0.001);
                    if all_same && !mesh.positions.is_empty() {
                        // Generate planar UVs from positions (bounding box normalized)
                        tc_data = generate_planar_uvs(&mesh.positions);
                        Some(tc_data.as_slice())
                    } else {
                        Some(tcs.as_slice())
                    }
                } else if !mesh.positions.is_empty() {
                    // No texcoords at all — generate planar UVs
                    tc_data = generate_planar_uvs(&mesh.positions);
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
                let buffers = Mesh3dBuffers::new(
                    context,
                    &mesh.positions,
                    &mesh.normals,
                    tc,
                    tc2,
                    &mesh.faces,
                )?;
                group.push(buffers);
            }
            mesh_groups.insert(name.clone(), group);
        }

        // Upload raw meshes
        for mesh in &scene.raw_meshes {
            if mesh.positions.is_empty() || mesh.faces.is_empty() {
                continue;
            }
            let tc = if !mesh.tex_coords.is_empty() {
                Some(mesh.tex_coords.as_slice())
            } else {
                None
            };
            let buffers = Mesh3dBuffers::new(
                context,
                &mesh.positions,
                &mesh.normals,
                tc,
                None, // raw meshes don't have 2nd UV set
                &mesh.faces,
            )?;
            all_meshes.push(buffers);
        }

        // Upload textures (decode JPEG/PNG or raw RGBA)
        // Store with lowercase keys for case-insensitive lookup
        let mut textures = HashMap::new();
        let mut map_tex_log = 0u32;
        for (tex_name, image_data) in &scene.texture_images {
            // Log "Map #" texture data sizes
            if map_tex_log < 8 && tex_name.to_lowercase().starts_with("map #") {
                web_sys::console::log_1(&format!(
                    "[3D-TEX] '{}': {} bytes raw data",
                    tex_name, image_data.len()
                ).into());
                map_tex_log += 1;
            }
            if let Some(tex) = self.decode_and_upload_texture(context, image_data) {
                textures.insert(tex_name.to_lowercase(), tex);
            }
        }

        self.member_data.insert(key, MemberGpuData { mesh_groups, all_meshes, textures, scene_version: current_version });
        Ok(())
    }

    /// Decode JPEG/PNG image data and upload as WebGL texture
    fn decode_and_upload_texture(&self, context: &WebGL2Context, data: &[u8]) -> Option<WebGlTexture> {
        // Check for raw RGBA format (from newTexture #fromImageObject):
        // first 4 bytes = width LE, next 4 bytes = height LE, rest = RGBA
        let (width, height, rgba_data) = if data.len() >= 8 {
            let w = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
            let h = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
            let expected = 8 + (w as usize) * (h as usize) * 4;
            if w > 0 && w <= 4096 && h > 0 && h <= 4096 && data.len() == expected {
                // Raw RGBA from Lingo image
                (w, h, data[8..].to_vec())
            } else {
                // Try JPEG/PNG decode
                let img = match image::load_from_memory(data) {
                    Ok(img) => img.to_rgba8(),
                    Err(e) => {
                        // Log first few bytes to diagnose format
                        let header: Vec<String> = data.iter().take(8).map(|b| format!("{:02X}", b)).collect();
                        web_sys::console::warn_1(&format!(
                            "[3D-TEX-DECODE] Failed to decode {} bytes, header=[{}]: {}",
                            data.len(), header.join(" "), e
                        ).into());
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
            web_sys::console::error_1(&format!(
                "[3D-TEX] Size mismatch! {}x{} expects {} bytes but got {}",
                width, height, expected_size, rgba_data.len()
            ).into());
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
            web_sys::console::error_1(&format!("[3D-TEX] tex_image_2d failed: {:?}", e).into());
        }
        gl.generate_mipmap(WebGl2RenderingContext::TEXTURE_2D);
        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, None);
        Some(texture)
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
        let projection_matrix = self.build_projection_matrix(scene, width as f32 / height as f32);

        gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &view_matrix);
        gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &projection_matrix);
        gl.uniform3f(shader.u_camera_pos.as_ref(), camera_pos[0], camera_pos[1], camera_pos[2]);

        self.setup_lights(gl, shader, scene, &camera_pos);
        gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
        gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
        gl.uniform1i(shader.u_has_texcoord2.as_ref(), 0);

        // Draw all meshes with proper material/texture binding
        if let Some(gpu_data) = self.member_data.get(&member_key) {
            let identity = [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0];
            gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &identity);

            // Try to find and bind material + texture from scene shaders
            let mut tex_bound = false;
            if let Some(mat) = scene.materials.iter().find(|m| !m.name.contains("Default")) {
                self.set_material_uniforms(gl, shader, mat);
            } else {
                self.bind_default_material(gl, shader, scene);
            }

            for w3d_shader in &scene.shaders {
                if tex_bound { break; }
                for layer in &w3d_shader.texture_layers {
                    if !layer.name.is_empty() {
                        if let Some(tex) = gpu_data.textures.get(&layer.name.to_lowercase()) {
                            gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
                            gl.uniform1i(shader.u_has_texture.as_ref(), 1);
                            gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
                            tex_bound = true;
                            break;
                        }
                    }
                }
            }
            if !tex_bound {
                gl.uniform1i(shader.u_has_texture.as_ref(), 0);
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

        let gl = context.gl();
        let shader = self.shader.as_ref().unwrap();
        let fbo = self.fbo.as_ref().unwrap();

        // Bind FBO
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, Some(fbo));
        gl.viewport(0, 0, width as i32, height as i32);

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
        let projection_matrix = self.build_projection_matrix(scene, width as f32 / height as f32);

        gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &view_matrix);
        gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &projection_matrix);
        gl.uniform3f(shader.u_camera_pos.as_ref(), camera_pos[0], camera_pos[1], camera_pos[2]);

        // Log camera info once
        {
            use std::sync::atomic::{AtomicBool, Ordering};
            static LOGGED: AtomicBool = AtomicBool::new(false);
            if !LOGGED.swap(true, Ordering::Relaxed) {
                web_sys::console::log_1(&format!(
                    "[3D-VIEW] camera_pos=({:.1},{:.1},{:.1}) view_mat=[{:.3},{:.3},{:.3},{:.3}, {:.3},{:.3},{:.3},{:.3}, {:.3},{:.3},{:.3},{:.3}, {:.3},{:.3},{:.3},{:.3}]",
                    camera_pos[0], camera_pos[1], camera_pos[2],
                    view_matrix[0], view_matrix[1], view_matrix[2], view_matrix[3],
                    view_matrix[4], view_matrix[5], view_matrix[6], view_matrix[7],
                    view_matrix[8], view_matrix[9], view_matrix[10], view_matrix[11],
                    view_matrix[12], view_matrix[13], view_matrix[14], view_matrix[15],
                ).into());
            }
        }

        // Set up lighting (pass camera pos for headlight direction)
        self.setup_lights(gl, shader, scene, &camera_pos);

        // Set texture samplers
        gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);   // unit 0 = base/diffuse
        gl.uniform1i(shader.u_lightmap_tex.as_ref(), 1);   // unit 1 = shadow/lightmap
        gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);   // default: no lightmap

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

            // Apply background color override
            if let Some((r, g, b)) = rs.background_color {
                gl.clear_color(r as f32 / 255.0, g as f32 / 255.0, b as f32 / 255.0, 1.0);
                gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT | WebGl2RenderingContext::DEPTH_BUFFER_BIT);
            }
        } else {
            gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
        }

        // Skinning defaults to off (enabled when bone data is present)
        gl.uniform1i(shader.u_has_texcoord2.as_ref(), 0);

        // Traverse scene graph and draw model nodes
        if self.member_data.contains_key(&member_key) {
            let model_nodes: Vec<_> = scene.nodes.iter()
                .filter(|n| n.node_type == W3dNodeType::Model)
                .cloned()
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
                web_sys::console::log_1(&format!(
                    "[3D] Scene {:?}: {} model_nodes={:?}, mesh_groups={:?}, textures={}",
                    member_key, model_nodes.len(), model_names, mesh_group_keys,
                    gpu_data.map(|d| d.textures.len()).unwrap_or(0)
                ).into());
                // Log motion data
                for (mi, motion) in scene.motions.iter().enumerate() {
                    web_sys::console::log_1(&format!(
                        "[3D-MOTION] motion[{}] '{}': {} tracks, duration={:.2}s",
                        mi, motion.name, motion.tracks.len(), motion.duration()
                    ).into());
                    for track in &motion.tracks {
                        let node_type = scene.nodes.iter().find(|n| n.name == track.bone_name)
                            .map(|n| format!("{:?}", n.node_type)).unwrap_or("NOT_FOUND".into());
                        web_sys::console::log_1(&format!(
                            "[3D-MOTION]   track '{}' ({}) {} keyframes, first_kf=[pos=({:.1},{:.1},{:.1}) rot=({:.3},{:.3},{:.3},{:.3})]",
                            track.bone_name, node_type, track.keyframes.len(),
                            track.keyframes.first().map(|k| k.pos_x).unwrap_or(0.0),
                            track.keyframes.first().map(|k| k.pos_y).unwrap_or(0.0),
                            track.keyframes.first().map(|k| k.pos_z).unwrap_or(0.0),
                            track.keyframes.first().map(|k| k.rot_x).unwrap_or(0.0),
                            track.keyframes.first().map(|k| k.rot_y).unwrap_or(0.0),
                            track.keyframes.first().map(|k| k.rot_z).unwrap_or(0.0),
                            track.keyframes.first().map(|k| k.rot_w).unwrap_or(1.0),
                        ).into());
                    }
                }
            }

            // Evaluate motion animations each frame
            self.motion_transforms.clear();
            if !scene.motions.is_empty() {
                self.animation_time += 1.0 / 30.0;
                for motion in &scene.motions {
                    let duration = motion.duration();
                    if duration <= 0.0 { continue; }
                    let t = self.animation_time % duration;
                    for track in &motion.tracks {
                        let mut kf = track.evaluate(t);
                        // Fix zero scale (motion data may not include scale → defaults to 0)
                        if kf.scale_x.abs() < 1e-6 { kf.scale_x = 1.0; }
                        if kf.scale_y.abs() < 1e-6 { kf.scale_y = 1.0; }
                        if kf.scale_z.abs() < 1e-6 { kf.scale_z = 1.0; }
                        let m = keyframe_to_column_major_matrix(&kf);
                        if self.animation_time < 0.1 {
                            web_sys::console::log_1(&format!(
                                "[3D-ANIM] t={:.3} node='{}' pos=({:.2},{:.2},{:.2}) rot=({:.3},{:.3},{:.3},{:.3}) scale=({:.2},{:.2},{:.2})",
                                t, track.bone_name, kf.pos_x, kf.pos_y, kf.pos_z,
                                kf.rot_x, kf.rot_y, kf.rot_z, kf.rot_w,
                                kf.scale_x, kf.scale_y, kf.scale_z
                            ).into());
                        }
                        self.motion_transforms.insert(track.bone_name.clone(), m);
                    }
                }
            }

            if model_nodes.is_empty() {
                // No model nodes — fallback: draw all meshes with identity transform
                self.draw_all_meshes_fallback(gl, shader, scene, &member_key);
            } else {
                let mut draw_stats = (0u32, 0u32, 0u32, 0u32); // (drawn, textured, no_tex, no_mesh)
                // Walk model nodes with accumulated transforms
                for model_node in &model_nodes {
                    // Check visibility override
                    if let Some(rs) = runtime_state {
                        if let Some(&visible) = rs.node_visibility.get(&model_node.name) {
                            if !visible { continue; }
                        }
                    }

                    let world_matrix = self.accumulate_transform_with_state(scene, model_node, runtime_state);
                    // Director transforms are already column-major — pass directly to GL
                    gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &world_matrix);

                    // Find mesh resource matching this model node
                    let resource = if !model_node.model_resource_name.is_empty() {
                        &model_node.model_resource_name
                    } else {
                        &model_node.resource_name
                    };

                    // Get per-mesh shader bindings from model resource
                    let res_info = scene.model_resources.get(resource);

                    if let Some(gpu_data) = self.member_data.get(&member_key) {
                        if let Some(mesh_group) = gpu_data.mesh_groups.get(resource) {
                            for (mesh_idx, mesh_buf) in mesh_group.iter().enumerate() {
                                let bound = self.bind_material_for_mesh(
                                    gl, shader, scene, model_node,
                                    res_info, mesh_idx, &member_key, runtime_state,
                                );
                                if !bound {
                                    self.bind_material(gl, shader, scene, model_node, &member_key, runtime_state);
                                }

                                mesh_buf.bind(gl);
                                mesh_buf.draw(gl);
                                mesh_buf.unbind(gl);
                                draw_stats.0 += 1;
                            }
                        } else {
                            draw_stats.3 += 1;
                        }
                    }
                }


            }
        }

        // Render particles (after opaque geometry, with additive blending)
        let _ = self.render_particles(context, runtime_state, &view_matrix, &projection_matrix);

        // Render camera overlays (2D textures on top of 3D scene)
        {
            let shader_ref = self.shader.as_ref().unwrap();
            if let Some(rs) = runtime_state {
                let cam_name = self.active_camera.clone();
                if let Some(ref cam) = cam_name {
                    if let Some(overlays) = rs.camera_overlays.get(cam.as_str()) {
                        if !overlays.is_empty() {
                            if let Some(gpu_data) = self.member_data.get(&member_key) {
                                Self::render_overlays_static(gl, shader_ref, overlays, gpu_data, width, height);
                            }
                        }
                    }
                }
            }
        }

        // Restore state
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.bind_framebuffer(WebGl2RenderingContext::FRAMEBUFFER, None);

        Ok(self.fbo_texture.as_ref())
    }

    /// Render camera overlays as 2D textured quads on top of the 3D scene
    fn render_overlays_static(
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        overlays: &[crate::player::cast_member::CameraOverlay],
        gpu_data: &MemberGpuData,
        width: u32,
        height: u32,
    ) {

        // Switch to orthographic 2D projection (pixel coordinates, origin top-left)
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);

        // Ortho projection: map (0,0)-(width,height) to clip space
        let w = width as f32;
        let h = height as f32;
        let ortho: [f32; 16] = [
            2.0/w,  0.0,    0.0, 0.0,
            0.0,   -2.0/h,  0.0, 0.0,
            0.0,    0.0,   -1.0, 0.0,
           -1.0,    1.0,    0.0, 1.0,
        ];
        let identity: [f32; 16] = [
            1.0, 0.0, 0.0, 0.0,
            0.0, 1.0, 0.0, 0.0,
            0.0, 0.0, 1.0, 0.0,
            0.0, 0.0, 0.0, 1.0,
        ];

        gl.uniform_matrix4fv_with_f32_array(shader.u_projection.as_ref(), false, &ortho);
        gl.uniform_matrix4fv_with_f32_array(shader.u_view.as_ref(), false, &identity);
        // Disable lighting for overlays
        gl.uniform1i(shader.u_num_lights.as_ref(), 0);
        gl.uniform1i(shader.u_fog_enabled.as_ref(), 0);
        gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);
        gl.uniform4f(shader.u_emissive_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
        gl.uniform4f(shader.u_diffuse_color.as_ref(), 1.0, 1.0, 1.0, 1.0);
        gl.uniform4f(shader.u_ambient_color.as_ref(), 0.0, 0.0, 0.0, 1.0);

        for overlay in overlays {
            if overlay.source_texture.is_empty() || overlay.blend <= 0.0 { continue; }

            let tex = gpu_data.textures.get(&overlay.source_texture.to_lowercase());
            if tex.is_none() { continue; }
            let tex = tex.unwrap();

            // Bind texture
            gl.active_texture(WebGl2RenderingContext::TEXTURE0);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
            gl.uniform1i(shader.u_diffuse_tex.as_ref(), 0);
            gl.uniform1i(shader.u_has_texture.as_ref(), 1);

            // Set opacity from blend (0-100 -> 0.0-1.0)
            gl.uniform1f(shader.u_opacity.as_ref(), (overlay.blend / 100.0) as f32);

            // Compute quad model matrix from overlay properties
            let x = overlay.loc[0] as f32;
            let y = overlay.loc[1] as f32;
            let sx = (overlay.scale * overlay.scale_x) as f32;
            let sy = (overlay.scale * overlay.scale_y) as f32;
            let rx = overlay.reg_point[0] as f32;
            let ry = overlay.reg_point[1] as f32;

            // Get texture dimensions for quad size (default 64x64)
            // We'll use a unit quad scaled by texture size
            let tex_w = 64.0f32; // TODO: track actual texture dimensions
            let tex_h = 64.0f32;

            // Build model matrix: translate to loc, apply scale, offset by regPoint
            let model: [f32; 16] = [
                sx * tex_w, 0.0,        0.0, 0.0,
                0.0,        sy * tex_h, 0.0, 0.0,
                0.0,        0.0,        1.0, 0.0,
                x - rx * sx, y - ry * sy, 0.0, 1.0,
            ];
            gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &model);

            // Draw a quad (two triangles) using gl.draw_arrays
            // We need a simple quad VAO — reuse positions from a temporary buffer
            let verts: [f32; 36] = [
                // pos(x,y,z) + normal(0,0,1) for 2 triangles
                0.0, 0.0, 0.0,  0.0, 0.0, 1.0,
                1.0, 0.0, 0.0,  0.0, 0.0, 1.0,
                1.0, 1.0, 0.0,  0.0, 0.0, 1.0,
                0.0, 0.0, 0.0,  0.0, 0.0, 1.0,
                1.0, 1.0, 0.0,  0.0, 0.0, 1.0,
                0.0, 1.0, 0.0,  0.0, 0.0, 1.0,
            ];
            let uvs: [f32; 12] = [
                0.0, 0.0,
                1.0, 0.0,
                1.0, 1.0,
                0.0, 0.0,
                1.0, 1.0,
                0.0, 1.0,
            ];

            let vert_buf = gl.create_buffer();
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, vert_buf.as_ref());
            unsafe {
                let vert_view = js_sys::Float32Array::view(&verts);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER, &vert_view, WebGl2RenderingContext::STREAM_DRAW);
            }
            gl.enable_vertex_attrib_array(0); // a_position
            gl.vertex_attrib_pointer_with_i32(0, 3, WebGl2RenderingContext::FLOAT, false, 24, 0);
            gl.enable_vertex_attrib_array(1); // a_normal
            gl.vertex_attrib_pointer_with_i32(1, 3, WebGl2RenderingContext::FLOAT, false, 24, 12);

            let uv_buf = gl.create_buffer();
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, uv_buf.as_ref());
            unsafe {
                let uv_view = js_sys::Float32Array::view(&uvs);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER, &uv_view, WebGl2RenderingContext::STREAM_DRAW);
            }
            gl.enable_vertex_attrib_array(2); // a_texcoord
            gl.vertex_attrib_pointer_with_i32(2, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);

            gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 6);

            gl.delete_buffer(vert_buf.as_ref());
            gl.delete_buffer(uv_buf.as_ref());
        }
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
        // Get this node's transform: motion (combined with base), runtime override, or parsed
        let node_transform = if let Some(motion_t) = self.motion_transforms.get(&node.name) {
            // Motion applied to base: motion * base (motion rotates the base orientation)
            mat4_multiply_col_major(motion_t, &node.transform)
        } else {
            runtime_state
                .and_then(|rs| rs.node_transforms.get(&node.name))
                .copied()
                .unwrap_or(node.transform)
        };

        let mut chain = vec![node_transform];
        let mut current_parent = &node.parent_name;

        // Walk up parent chain
        while !current_parent.is_empty() && current_parent != "<world>" {
            if let Some(parent_node) = scene.nodes.iter().find(|n| n.name == *current_parent) {
                let parent_t = runtime_state
                    .and_then(|rs| rs.node_transforms.get(&parent_node.name))
                    .copied()
                    .unwrap_or(parent_node.transform);
                chain.push(parent_t);
                current_parent = &parent_node.parent_name;
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

    /// Find the best texture layers: returns (diffuse, optional shadow/lightmap, blend_mode)
    /// Returns (diffuse_tex, secondary_tex, blend_mode, intensity)
    /// blend_mode: 0=none, 1=multiply(shadow), 2=add(lightmap)
    fn find_texture_layers<'a>(
        layers: &[crate::director::chunks::w3d::types::W3dTextureLayer],
        gpu_data: &'a MemberGpuData,
    ) -> (Option<&'a WebGlTexture>, Option<&'a WebGlTexture>, i32, f32) {
        let mut diffuse: Option<&'a WebGlTexture> = None;
        let mut secondary: Option<&'a WebGlTexture> = None;
        let mut blend_mode: i32 = 0;
        let mut intensity: f32 = 1.0;

        for layer in layers {
            if layer.name.is_empty() { continue; }
            if let Some(tex) = gpu_data.textures.get(&layer.name.to_lowercase()) {
                if diffuse.is_none() {
                    // First texture becomes diffuse
                    diffuse = Some(tex);
                } else if secondary.is_none() {
                    // Second texture becomes secondary with blend from layer data
                    secondary = Some(tex);
                    intensity = layer.intensity;
                    // IFX texture combine modes (from PrepareDiffusePass):
                    //   blend_func 0 = GL_REPLACE (7681) → just texture, no lighting
                    //   blend_func 1 = GL_ADD (260)      → texture + incoming
                    //   blend_func 2 = GL_MODULATE (8448) → texture × incoming (multiply)
                    //   blend_func 3+ = GL_DECAL (8449)
                    // Map to our blend_mode: 1=multiply, 2=add
                    blend_mode = match layer.blend_func {
                        1 => 2,  // IFX ADD → our add mode
                        2 => 1,  // IFX MODULATE → our multiply mode
                        _ => 1,  // default to multiply
                    };
                    // Name-based fallback
                    let lower = layer.name.to_lowercase();
                    if lower.contains("lightmap") && !lower.contains("shadow") {
                        blend_mode = 2; // add for lightmaps
                    }
                }
            }
        }

        // If no diffuse found, use shadow/light as diffuse
        if diffuse.is_none() && secondary.is_some() {
            diffuse = secondary.take();
            blend_mode = 0;
        }

        (diffuse, secondary, blend_mode, intensity)
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
            .and_then(|rs| rs.node_shaders.get(&model_node.name))
            .cloned()
            .unwrap_or_else(|| model_node.shader_name.clone());

        if !effective_shader_name.is_empty() {
            if let Some(w3d_shader) = scene.shaders.iter().find(|s| s.name == effective_shader_name) {
                // Find material: try shader's material_name, then shader name itself
                let mat = if !w3d_shader.material_name.is_empty() {
                    scene.materials.iter().find(|m| m.name == w3d_shader.material_name)
                } else { None }
                    .or_else(|| scene.materials.iter().find(|m| m.name == w3d_shader.name));
                if let Some(mat) = mat {
                    self.set_material_uniforms(gl, shader, mat);
                    mat_found = true;
                }

                // Bind texture layers: diffuse + optional shadow/lightmap
                if let Some(gpu_data) = self.member_data.get(member_key) {
                    let (diffuse, secondary, blend_mode, lm_intensity) = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                    if let Some(tex) = diffuse {
                        gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
                        gl.uniform1i(shader.u_has_texture.as_ref(), 1);
                        tex_bound = true;
                    }
                    if let Some(lm) = secondary {
                        gl.active_texture(WebGl2RenderingContext::TEXTURE1);
                        gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(lm));
                        gl.uniform1i(shader.u_has_lightmap.as_ref(), blend_mode);
                        gl.uniform1f(shader.u_lightmap_intensity.as_ref(), lm_intensity);
                    } else {
                        gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);
                    }
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
                    if let Some(w3d_shader) = scene.shaders.iter().find(|s| s.name == binding.name) {
                        if let Some(mat) = scene.materials.iter().find(|m| m.name == w3d_shader.material_name) {
                            self.set_material_uniforms(gl, shader, mat);
                            mat_found = true;
                        }
                        // Bind texture layers from shader binding
                        if !tex_bound {
                            if let Some(gpu_data) = self.member_data.get(member_key) {
                                let (diffuse, secondary, blend_mode, lm_intensity) = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                                if let Some(tex) = diffuse {
                                    gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
                                    gl.uniform1i(shader.u_has_texture.as_ref(), 1);
                                    tex_bound = true;
                                }
                                if let Some(lm) = secondary {
                                    gl.active_texture(WebGl2RenderingContext::TEXTURE1);
                                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(lm));
                                    gl.uniform1i(shader.u_has_lightmap.as_ref(), blend_mode);
                        gl.uniform1f(shader.u_lightmap_intensity.as_ref(), lm_intensity);
                                } else {
                                    gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);
                                }
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
    }

    /// Bind material for a specific mesh index using model resource shader bindings
    fn bind_material_for_mesh(
        &self,
        gl: &WebGl2RenderingContext,
        shader: &Shader3d,
        scene: &W3dScene,
        _model_node: &W3dNode,
        res_info: Option<&ModelResourceInfo>,
        mesh_idx: usize,
        member_key: &(i32, i32),
        _runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> bool {
        let res_info = match res_info {
            Some(r) => r,
            None => return false,
        };

        // Walk shader bindings - prefer bindings that have textures
        // Each binding has: name (shader name) + mesh_bindings (per-mesh shader/material overrides)
        let mut best_material: Option<&W3dMaterial> = None;

        for binding in &res_info.shader_bindings {
            // Resolve shader: try mesh-specific binding name first, then top-level binding name
            let mesh_binding_name = if mesh_idx < binding.mesh_bindings.len() {
                &binding.mesh_bindings[mesh_idx]
            } else {
                &binding.name
            };
            let w3d_shader = if !mesh_binding_name.is_empty() {
                scene.shaders.iter().find(|s| s.name == *mesh_binding_name)
            } else {
                None
            }.or_else(|| scene.shaders.iter().find(|s| s.name == binding.name));

            if w3d_shader.is_none() { continue; }
            let w3d_shader = w3d_shader.unwrap();

            // Get material: try multiple lookup strategies
            let mesh_bind_name = if mesh_idx < binding.mesh_bindings.len() && !binding.mesh_bindings[mesh_idx].is_empty() {
                Some(&binding.mesh_bindings[mesh_idx])
            } else {
                None
            };
            let mat = mesh_bind_name.and_then(|n| scene.materials.iter().find(|m| m.name == *n))
                .or_else(|| if !w3d_shader.material_name.is_empty() {
                    scene.materials.iter().find(|m| m.name == w3d_shader.material_name)
                } else { None })
                .or_else(|| scene.materials.iter().find(|m| m.name == w3d_shader.name));

            // Try binding texture layers from this shader
            let mut tex_bound = false;
            if let Some(gpu_data) = self.member_data.get(member_key) {
                let (diffuse, secondary, blend_mode, lm_intensity) = Self::find_texture_layers(&w3d_shader.texture_layers, gpu_data);
                if let Some(tex) = diffuse {
                    gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
                    gl.uniform1i(shader.u_has_texture.as_ref(), 1);
                    tex_bound = true;
                }
                if let Some(lm) = secondary {
                    gl.active_texture(WebGl2RenderingContext::TEXTURE1);
                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(lm));
                    gl.uniform1i(shader.u_has_lightmap.as_ref(), blend_mode);
                        gl.uniform1f(shader.u_lightmap_intensity.as_ref(), lm_intensity);
                } else {
                    gl.uniform1i(shader.u_has_lightmap.as_ref(), 0);
                }
            }

            // If this binding has a texture, use it immediately (best match)
            if tex_bound {
                if let Some(m) = mat {
                    self.set_material_uniforms(gl, shader, m);
                }
                return true;
            }

            // Otherwise remember the material for fallback
            if best_material.is_none() {
                best_material = mat;
            }
        }

        // No textured binding found — use best material without texture
        if let Some(mat) = best_material {
            self.set_material_uniforms(gl, shader, mat);
            gl.uniform1i(shader.u_has_texture.as_ref(), 0);
            return true;
        }

        false
    }

    fn set_material_uniforms(&self, gl: &WebGl2RenderingContext, shader: &Shader3d, mat: &W3dMaterial) {
        gl.uniform4f(shader.u_diffuse_color.as_ref(), mat.diffuse[0], mat.diffuse[1], mat.diffuse[2], mat.diffuse[3]);
        gl.uniform4f(shader.u_ambient_color.as_ref(), mat.ambient[0], mat.ambient[1], mat.ambient[2], mat.ambient[3]);
        gl.uniform4f(shader.u_specular_color.as_ref(), mat.specular[0], mat.specular[1], mat.specular[2], mat.specular[3]);
        gl.uniform4f(shader.u_emissive_color.as_ref(), mat.emissive[0], mat.emissive[1], mat.emissive[2], mat.emissive[3]);
        gl.uniform1f(shader.u_shininess.as_ref(), mat.shininess);
        gl.uniform1f(shader.u_opacity.as_ref(), mat.opacity);
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

    /// Build view matrix from scene's ViewNode (or default camera)
    fn build_view_matrix(
        &self,
        scene: &W3dScene,
        runtime_state: Option<&crate::player::cast_member::Shockwave3dRuntimeState>,
    ) -> ([f32; 16], [f32; 3]) {
        // 1. Determine which camera to use
        let default_cam = "DefaultView".to_string();
        let cam_name = self.active_camera.as_ref().unwrap_or(&default_cam);

        // 2. Find the camera node and accumulate its full world transform (including parent chain)
        if let Some(node) = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View && n.name == *cam_name) {
            let world_t = self.accumulate_transform_with_state(scene, node, runtime_state);
            let cam_pos = [world_t[12], world_t[13], world_t[14]];
            return (invert_transform(&world_t), cam_pos);
        }
        // Fallback: try any view node
        let view_node = scene.nodes.iter().find(|n| n.node_type == W3dNodeType::View);
        let cam_name = view_node.map(|n| n.name.as_str()).unwrap_or("DefaultView");

        // 3. Check runtime transform for this camera
        if let Some(rs) = runtime_state {
            if let Some(cam_t) = rs.node_transforms.get(cam_name) {
                let cam_pos = [cam_t[12], cam_t[13], cam_t[14]];
                return (invert_transform(cam_t), cam_pos);
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
    fn build_projection_matrix(&self, scene: &W3dScene, _fbo_aspect: f32) -> [f32; 16] {
        // Use the same camera as build_view_matrix (last view node)
        let view_node = scene.nodes.iter()
            .filter(|n| n.node_type == W3dNodeType::View)
            .last();

        let (fov, near, far, aspect) = if let Some(node) = view_node {
            let mut f = node.far_plane;
            // Clamp far plane: f32::MAX or huge values destroy depth buffer precision
            if f > 100000.0 || f <= 0.0 { f = 10000.0; }
            let mut n = node.near_plane;
            if n <= 0.0 { n = 1.0; }
            // Use camera's screen dimensions for aspect ratio (matches Director's camera.rect)
            let cam_aspect = if node.screen_width > 0 && node.screen_height > 0 {
                node.screen_width as f32 / node.screen_height as f32
            } else {
                _fbo_aspect
            };
            (node.fov.to_radians(), n, f, cam_aspect)
        } else {
            (45.0f32.to_radians(), 1.0, 10000.0, _fbo_aspect)
        };

        let mut proj = perspective(fov, aspect, near, far);
        // Flip Y: FBO renders with OpenGL Y-up but composited as 2D sprite with Y-down
        proj[5] = -proj[5];
        proj
    }

    /// Set up lighting uniforms from scene lights
    fn setup_lights(&self, gl: &WebGl2RenderingContext, shader: &Shader3d, scene: &W3dScene, camera_pos: &[f32; 3]) {
        let mut positions = [0.0f32; 24]; // 8 * 3
        let mut colors = [0.0f32; 24];
        let mut types = [0i32; 8];
        let mut global_ambient = [0.2f32, 0.2, 0.2];
        let mut num_lights = 0i32;

        if scene.lights.is_empty() {
            // Default: one directional light from above-right
            positions[0] = 0.5;
            positions[1] = 1.0;
            positions[2] = 0.7;
            colors[0] = 1.0;
            colors[1] = 1.0;
            colors[2] = 1.0;
            types[0] = 1; // directional
            num_lights = 1;
        } else {
            for (i, light) in scene.lights.iter().enumerate() {
                if i >= 8 || !light.enabled {
                    continue;
                }
                let li = num_lights as usize;
                let lt = match light.light_type {
                    W3dLightType::Ambient => {
                        global_ambient[0] += light.color[0];
                        global_ambient[1] += light.color[1];
                        global_ambient[2] += light.color[2];
                        continue; // ambient lights don't count as per-light
                    }
                    W3dLightType::Directional => 1,
                    W3dLightType::Point => 2,
                    W3dLightType::Spot => 3,
                };
                if let Some(light_node) = scene.nodes.iter().find(|n| {
                    n.node_type == W3dNodeType::Light && (n.resource_name == light.name || n.name == light.name)
                }) {
                    // Get light's world transform (accumulated through parent chain)
                    let world_t = self.accumulate_transform_with_state(scene, light_node, None);
                    if lt == 1 {
                        // Directional: direction = -Z axis (column 2) of world transform
                        positions[li * 3]     = -world_t[8];
                        positions[li * 3 + 1] = -world_t[9];
                        positions[li * 3 + 2] = -world_t[10];
                    } else {
                        // Point/Spot: world position from translation
                        positions[li * 3]     = world_t[12];
                        positions[li * 3 + 1] = world_t[13];
                        positions[li * 3 + 2] = world_t[14];
                    }
                } else {
                    // Default: light from above-right diagonal
                    positions[li * 3] = 0.5;
                    positions[li * 3 + 1] = 1.0;
                    positions[li * 3 + 2] = 0.7;
                }
                colors[li * 3] = light.color[0];
                colors[li * 3 + 1] = light.color[1];
                colors[li * 3 + 2] = light.color[2];
                types[li] = lt;
                num_lights += 1;
            }
        }

        gl.uniform1i(shader.u_num_lights.as_ref(), num_lights);
        gl.uniform3fv_with_f32_array(shader.u_light_pos.as_ref(), &positions[..num_lights.max(1) as usize * 3]);
        gl.uniform3fv_with_f32_array(shader.u_light_color.as_ref(), &colors[..num_lights.max(1) as usize * 3]);
        gl.uniform1iv_with_i32_array(shader.u_light_type.as_ref(), &types[..num_lights.max(1) as usize]);
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
