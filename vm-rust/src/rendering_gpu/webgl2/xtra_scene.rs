// SPDX-License-Identifier: GPL-3.0-only
//
//! Engine-agnostic WebGL2 renderer for the external Xtra 3D scene API.
//!
//! Reads the CPU-side [`Scene3dStore`](crate::player::xtra::scene3d) that
//! external plugins fill through the `scene3d_*` host services and composites
//! each scene onto the dirplayer stage. Unlike the built-in
//! `GrooveSceneRenderer`, this holds no engine knowledge: a plugin uploads
//! flat-shaded triangle batches once (per shape), then each step submits a
//! [`FrameData`](xtra_sdk::scene3d::FrameData) of draw commands carrying
//! *final* model matrices + material — the plugin evaluates its own animation.
//!
//! GL objects (buffers, textures) can't live in the store, so they're cached
//! here keyed by `(scene_id, mesh_id)` / `(scene_id, texture_name)`, rebuilt
//! only when the store's per-item `generation` counter bumps.

use std::collections::HashMap;

use web_sys::{WebGl2RenderingContext, WebGlProgram, WebGlTexture, WebGlUniformLocation};

use crate::player::cast_member::CastMemberType;
use crate::player::xtra::scene3d::with_store_mut;
use crate::player::DirPlayer;

use super::{context::WebGL2Context, mesh3d::Mesh3dBuffers};

const VS: &str = r#"#version 300 es
precision highp float;
layout(location=0) in vec3 a_pos;
layout(location=1) in vec3 a_normal;
layout(location=2) in vec2 a_uv;
uniform mat4 u_mvp;
uniform mat4 u_model;
out vec3 v_normal;
out vec2 v_uv;
void main() {
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    v_normal = mat3(u_model) * a_normal;
    v_uv = a_uv;
}
"#;

const FS: &str = r#"#version 300 es
precision highp float;
in vec3 v_normal;
in vec2 v_uv;
uniform vec3 u_light_dir;
uniform vec3 u_light_color;
uniform float u_ambient;
uniform vec3 u_color;
uniform sampler2D u_tex;
uniform int u_has_tex;
uniform float u_alpha;
out vec4 frag;
void main() {
    vec3 n = normalize(v_normal);
    float diff = max(abs(dot(n, normalize(u_light_dir))), 0.0);
    float shade = u_ambient + (1.0 - u_ambient) * diff;
    vec3 tex = u_has_tex == 1 ? texture(u_tex, v_uv).rgb : vec3(1.0);
    frag = vec4(tex * u_color * u_light_color * shade, u_alpha);
}
"#;

struct ShaderProgram {
    program: WebGlProgram,
    u_mvp: Option<WebGlUniformLocation>,
    u_model: Option<WebGlUniformLocation>,
    u_light_dir: Option<WebGlUniformLocation>,
    u_light_color: Option<WebGlUniformLocation>,
    u_ambient: Option<WebGlUniformLocation>,
    u_color: Option<WebGlUniformLocation>,
    u_tex: Option<WebGlUniformLocation>,
    u_has_tex: Option<WebGlUniformLocation>,
    u_alpha: Option<WebGlUniformLocation>,
}

/// One GL-uploaded batch: its texture name (empty = untextured) and buffers.
struct GlBatch {
    tex_name: String,
    mesh: Mesh3dBuffers,
}

/// Only composite while the game is actively driving the scene: it must have
/// submitted on the current movie frame AND within the last couple of draws.
const STALE_DRAWS: i32 = 2;

/// GL-side caches + shader. Lazily initialized.
pub struct XtraSceneRenderer {
    shader: Option<ShaderProgram>,
    /// Uploaded mesh buffers keyed by (scene_id, mesh_id): (store generation, batches).
    meshes: HashMap<(i32, u32), (u64, Vec<GlBatch>)>,
    /// GL textures keyed by (scene_id, name): (source generation, texture). For a
    /// plugin-uploaded texture the generation tracks the store entry; for a
    /// cast-member texture it's `CAST_TEX_GEN` (resolved once, no refresh).
    textures: HashMap<(i32, String), (u64, Option<WebGlTexture>)>,
}

/// Sentinel `generation` for cast-member-resolved textures (never auto-refreshed).
const CAST_TEX_GEN: u64 = u64::MAX;

impl XtraSceneRenderer {
    pub fn new() -> Self {
        XtraSceneRenderer { shader: None, meshes: HashMap::new(), textures: HashMap::new() }
    }

    fn ensure_shader(&mut self, context: &WebGL2Context) -> bool {
        if self.shader.is_some() {
            return true;
        }
        let gl = context.gl();
        let vs = match context.compile_shader(WebGl2RenderingContext::VERTEX_SHADER, VS) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let fs = match context.compile_shader(WebGl2RenderingContext::FRAGMENT_SHADER, FS) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let program = match context.link_program(&vs, &fs) {
            Ok(p) => p,
            Err(_) => return false,
        };
        let u = |n: &str| gl.get_uniform_location(&program, n);
        self.shader = Some(ShaderProgram {
            u_mvp: u("u_mvp"),
            u_model: u("u_model"),
            u_light_dir: u("u_light_dir"),
            u_light_color: u("u_light_color"),
            u_ambient: u("u_ambient"),
            u_color: u("u_color"),
            u_tex: u("u_tex"),
            u_has_tex: u("u_has_tex"),
            u_alpha: u("u_alpha"),
            program,
        });
        true
    }

    /// Composite every active scene in the store.
    pub fn draw(&mut self, context: &WebGL2Context, player: &DirPlayer, viewport_w: i32, viewport_h: i32) {
        // Collect the scene ids to draw, gating on staleness. We bump
        // `draws_since_submit` here (a store write) then release the store
        // borrow so the GL work below can freely re-borrow it read-only.
        let current_frame = player.movie.current_frame as i32;
        let scene_ids: Vec<i32> = with_store_mut(|store| {
            let mut ids = Vec::new();
            for (id, scene) in store.scenes.iter_mut() {
                if scene.frame.is_none() {
                    continue;
                }
                if scene.last_submit_frame != current_frame {
                    continue;
                }
                scene.draws_since_submit += 1;
                if scene.draws_since_submit > STALE_DRAWS {
                    continue;
                }
                ids.push(*id);
            }
            ids
        });
        if scene_ids.is_empty() {
            return;
        }
        if !self.ensure_shader(context) {
            return;
        }
        for scene_id in scene_ids {
            self.draw_scene(context, player, scene_id, viewport_w, viewport_h);
        }
    }

    fn draw_scene(
        &mut self,
        context: &WebGL2Context,
        player: &DirPlayer,
        scene_id: i32,
        viewport_w: i32,
        viewport_h: i32,
    ) {
        // Snapshot the frame + the mesh/texture upload work needed, all while
        // holding the store, then drop the borrow before issuing GL draws that
        // read `self` mutably. Cloning FrameData is cheap (draws are matrices).
        let frame = match with_store_mut(|store| {
            store.scenes.get(&scene_id).and_then(|s| s.frame.clone())
        }) {
            Some(f) => f,
            None => return,
        };
        if frame.draws.is_empty() {
            return;
        }

        // (1) Ensure GL meshes for every mesh referenced this frame.
        for d in &frame.draws {
            self.ensure_mesh(context, scene_id, d.mesh_id);
        }
        // (2) Ensure textures for every name referenced this frame.
        for d in &frame.draws {
            if let Some(name) = d.tex_override.as_deref().filter(|n| !n.is_empty()) {
                self.ensure_texture(context, player, scene_id, name);
            }
            if let Some((_, batches)) = self.meshes.get(&(scene_id, d.mesh_id)) {
                let names: Vec<String> = batches
                    .iter()
                    .map(|b| b.tex_name.clone())
                    .filter(|n| !n.is_empty())
                    .collect();
                for name in names {
                    self.ensure_texture(context, player, scene_id, &name);
                }
            }
        }

        let gl = context.gl();
        let (rx1, ry1, rx2, ry2) = frame.render_rect.unwrap_or((0, 0, viewport_w, viewport_h));
        let rect_w = (rx2 - rx1).max(1);
        let rect_h = (ry2 - ry1).max(1);

        // Map the render rect (stage px) to framebuffer px (may be HiDPI-scaled);
        // GL viewport origin is bottom-left so flip Y.
        let fb_w = gl.drawing_buffer_width();
        let fb_h = gl.drawing_buffer_height();
        let sx = fb_w as f32 / viewport_w.max(1) as f32;
        let sy = fb_h as f32 / viewport_h.max(1) as f32;
        let vp_x = (rx1 as f32 * sx).round() as i32;
        let vp_w = (rect_w as f32 * sx).round() as i32;
        let vp_h = (rect_h as f32 * sy).round() as i32;
        let vp_y = fb_h - (ry2 as f32 * sy).round() as i32;
        gl.viewport(vp_x, vp_y, vp_w, vp_h);
        gl.enable(WebGl2RenderingContext::SCISSOR_TEST);
        gl.scissor(vp_x, vp_y, vp_w, vp_h);

        gl.enable(WebGl2RenderingContext::DEPTH_TEST);
        gl.depth_func(WebGl2RenderingContext::LEQUAL);
        // A windowed view with a background clears opaque; a full-stage view
        // with no background composites over the 2D layer (depth clear only).
        match frame.background {
            Some(bg) => {
                gl.clear_color(bg[0] as f32 / 255.0, bg[1] as f32 / 255.0, bg[2] as f32 / 255.0, 1.0);
                gl.clear(WebGl2RenderingContext::COLOR_BUFFER_BIT | WebGl2RenderingContext::DEPTH_BUFFER_BIT);
            }
            None => gl.clear(WebGl2RenderingContext::DEPTH_BUFFER_BIT),
        }
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);

        let shader = self.shader.as_ref().unwrap();
        gl.use_program(Some(&shader.program));
        let (ldir, lcolor) = frame
            .light
            .map(|l| (l.dir, l.color))
            .unwrap_or(([0.3, 0.5, 1.0], [1.0, 1.0, 1.0]));
        gl.uniform3f(shader.u_light_dir.as_ref(), ldir[0], ldir[1], ldir[2]);
        gl.uniform3f(shader.u_light_color.as_ref(), lcolor[0], lcolor[1], lcolor[2]);
        gl.uniform1f(shader.u_ambient.as_ref(), frame.ambient.clamp(0.0, 1.0));

        let aspect = if rect_h > 0 { rect_w as f32 / rect_h as f32 } else { 1.0 };
        let proj = perspective(frame.camera.fov.to_radians().max(0.1), aspect, 1.0, 200_000.0);
        let view = look_at(frame.camera.pos, frame.camera.look_at, [0.0, 0.0, 1.0]);
        let view_proj = mat_mul(&proj, &view);

        for d in &frame.draws {
            let Some((_, batches)) = self.meshes.get(&(scene_id, d.mesh_id)) else { continue };
            let model = d.model;
            let mvp = mat_mul(&view_proj, &model);
            gl.uniform_matrix4fv_with_f32_array(shader.u_mvp.as_ref(), false, &mvp);
            gl.uniform_matrix4fv_with_f32_array(shader.u_model.as_ref(), false, &model);
            gl.uniform3f(
                shader.u_color.as_ref(),
                d.color[0] as f32 / 255.0,
                d.color[1] as f32 / 255.0,
                d.color[2] as f32 / 255.0,
            );
            let alpha = d.alpha.clamp(0.0, 1.0);
            gl.uniform1f(shader.u_alpha.as_ref(), alpha);
            gl.depth_mask(alpha >= 0.999);
            gl.uniform1i(shader.u_tex.as_ref(), 0);

            for (bi, batch) in batches.iter().enumerate() {
                if d.batch >= 0 && d.batch as usize != bi {
                    continue;
                }
                let tex_name = d
                    .tex_override
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(batch.tex_name.as_str());
                let tex = self
                    .textures
                    .get(&(scene_id, tex_name.to_string()))
                    .and_then(|(_, t)| t.as_ref());
                if let Some(t) = tex {
                    gl.active_texture(WebGl2RenderingContext::TEXTURE0);
                    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(t));
                    gl.uniform1i(shader.u_has_tex.as_ref(), 1);
                } else {
                    gl.uniform1i(shader.u_has_tex.as_ref(), 0);
                }
                batch.mesh.bind(gl);
                batch.mesh.draw(gl);
                batch.mesh.unbind(gl);
            }
        }
        gl.depth_mask(true);
        gl.disable(WebGl2RenderingContext::BLEND);
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::SCISSOR_TEST);
        gl.viewport(0, 0, fb_w, fb_h);
    }

    /// Build/refresh the GL buffers for one mesh if the store's generation advanced.
    fn ensure_mesh(&mut self, context: &WebGL2Context, scene_id: i32, mesh_id: u32) {
        let store_gen = with_store_mut(|store| {
            store.scenes.get(&scene_id).and_then(|s| s.meshes.get(&mesh_id)).map(|m| m.generation)
        });
        let Some(store_gen) = store_gen else { return };
        if self.meshes.get(&(scene_id, mesh_id)).map(|(g, _)| *g) == Some(store_gen) {
            return;
        }
        // Rebuild from the store's CPU batches.
        let batches = with_store_mut(|store| {
            let mut out: Vec<(String, Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>)> = Vec::new();
            if let Some(m) = store.scenes.get(&scene_id).and_then(|s| s.meshes.get(&mesh_id)) {
                for b in &m.data.batches {
                    out.push((
                        b.tex_name.clone(),
                        chunk3(&b.positions),
                        chunk3(&b.normals),
                        chunk2(&b.uvs),
                    ));
                }
            }
            out
        });
        let mut gl_batches = Vec::new();
        for (tex_name, positions, normals, uvs) in batches {
            let n_tris = positions.len() / 3;
            let faces: Vec<[u32; 3]> =
                (0..n_tris).map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32]).collect();
            let uvs_opt = if uvs.len() == positions.len() { Some(uvs.as_slice()) } else { None };
            if let Ok(mesh) =
                Mesh3dBuffers::new(context, &positions, &normals, uvs_opt, None, &faces)
            {
                gl_batches.push(GlBatch { tex_name, mesh });
            }
        }
        self.meshes.insert((scene_id, mesh_id), (store_gen, gl_batches));
    }

    /// Ensure a GL texture for `name` in `scene_id`. Prefers a plugin-uploaded
    /// RGBA texture (tracked by store generation); otherwise resolves `name` to a
    /// movie bitmap cast member and uploads it once.
    fn ensure_texture(&mut self, context: &WebGL2Context, player: &DirPlayer, scene_id: i32, name: &str) {
        // Plugin-uploaded texture? Check the store's generation.
        let uploaded = with_store_mut(|store| {
            store
                .scenes
                .get(&scene_id)
                .and_then(|s| s.textures.get(name))
                .map(|t| (t.generation, t.w, t.h, t.rgba.clone()))
        });
        let key = (scene_id, name.to_string());
        if let Some((generation, w, h, rgba)) = uploaded {
            if self.textures.get(&key).map(|(g, _)| *g) == Some(generation) {
                return; // up to date
            }
            let tex = upload_rgba(context, w, h, &rgba);
            self.textures.insert(key, (generation, tex));
            return;
        }
        // Cast-member texture: resolve once, cache with the sentinel generation.
        if self.textures.contains_key(&key) {
            return;
        }
        let tex = upload_named_texture(context, player, name);
        self.textures.insert(key, (CAST_TEX_GEN, tex));
    }
}

fn chunk3(v: &[f32]) -> Vec<[f32; 3]> {
    v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}
fn chunk2(v: &[f32]) -> Vec<[f32; 2]> {
    v.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
}

fn upload_rgba(context: &WebGL2Context, w: u32, h: u32, rgba: &[u8]) -> Option<WebGlTexture> {
    if w == 0 || h == 0 || rgba.len() != (w * h * 4) as usize {
        return None;
    }
    let tex = context.create_texture().ok()?;
    context.upload_texture_rgba(&tex, w, h, rgba).ok()?;
    Some(tex)
}

/// Resolve `name` to a movie bitmap cast member and upload it as a GL texture.
fn upload_named_texture(context: &WebGL2Context, player: &DirPlayer, name: &str) -> Option<WebGlTexture> {
    let mref = player.movie.cast_manager.find_member_ref_by_name(name)?;
    let member = player.movie.cast_manager.find_member_by_ref(&mref)?;
    let bref = match &member.member_type {
        CastMemberType::Bitmap(b) => b.image_ref,
        _ => return None,
    };
    let bitmap = player.bitmap_manager.get_bitmap(bref)?;
    upload_rgba(context, bitmap.width as u32, bitmap.height as u32, &bitmap.data)
}

// ---- minimal column-major mat4 helpers (GL order) ----

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

fn look_at(eye: [f32; 3], center: [f32; 3], up: [f32; 3]) -> [f32; 16] {
    let f = normalize(sub(center, eye));
    let s = normalize(cross(f, up));
    let u = cross(s, f);
    [
        s[0], u[0], -f[0], 0.0,
        s[1], u[1], -f[1], 0.0,
        s[2], u[2], -f[2], 0.0,
        -dot(s, eye), -dot(u, eye), dot(f, eye), 1.0,
    ]
}

fn mat_mul(a: &[f32; 16], b: &[f32; 16]) -> [f32; 16] {
    let mut o = [0.0f32; 16];
    for c in 0..4 {
        for r in 0..4 {
            let mut s = 0.0;
            for k in 0..4 {
                s += a[k * 4 + r] * b[c * 4 + k];
            }
            o[c * 4 + r] = s;
        }
    }
    o
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] { [a[0] - b[0], a[1] - b[1], a[2] - b[2]] }
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn dot(a: [f32; 3], b: [f32; 3]) -> f32 { a[0] * b[0] + a[1] * b[1] + a[2] * b[2] }
fn normalize(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 1e-8 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, 0.0, 1.0] }
}
