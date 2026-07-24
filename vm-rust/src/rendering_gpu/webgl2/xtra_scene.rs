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

use std::collections::{HashMap, HashSet};

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
layout(location=6) in vec4 a_color;
uniform mat4 u_mvp;
uniform mat4 u_model;
out vec3 v_normal;
out vec2 v_uv;
out vec4 v_color;
void main() {
    gl_Position = u_mvp * vec4(a_pos, 1.0);
    v_normal = mat3(u_model) * a_normal;
    v_uv = a_uv;
    v_color = a_color;
}
"#;

const FS: &str = r#"#version 300 es
precision highp float;
in vec3 v_normal;
in vec2 v_uv;
in vec4 v_color;
uniform vec3 u_light_dir;
uniform vec3 u_light_color;
uniform float u_ambient;
uniform vec3 u_color;
uniform sampler2D u_tex;
uniform int u_has_tex;
uniform int u_has_vcolor;
uniform float u_alpha;
out vec4 frag;
void main() {
    vec4 t = u_has_tex == 1 ? texture(u_tex, v_uv) : vec4(1.0);
    // Groove blit modes (`.t` transparent, `.g` greenscreen, `.s`+`.a` mask) are
    // decoded CPU-side into the texture's alpha. Drop keyed-out texels entirely
    // rather than blending them: a discarded fragment writes no depth, so the
    // scene behind a keyed background stays visible. Sampling only .rgb here (and
    // emitting u_alpha alone) made every keyed texture render opaque — black skies
    // and solid blocks where the cut-out should be.
    if (u_has_tex == 1 && t.a < 0.5) discard;
    // Base lit color (GL fixed-function-style: ambient floor + directional diffuse).
    vec3 n = normalize(v_normal);
    float diff = max(abs(dot(n, normalize(u_light_dir))), 0.0);
    float shade = u_ambient + (1.0 - u_ambient) * diff;
    vec3 lit = t.rgb * u_color * u_light_color * shade;
    // Material emission (self-illumination) is ADDED on top of the lit surface —
    // it does not replace lighting. Zero for non-emissive / non-material faces.
    vec3 emis = u_has_vcolor == 1 ? v_color.rgb : vec3(0.0);
    frag = vec4(lit + emis, u_alpha * t.a);
}
"#;

/// 2D overlay shader: a unit-quad corner (loc 0) mapped into an NDC rect, textured.
const OVERLAY_VS: &str = r#"#version 300 es
precision highp float;
layout(location=0) in vec2 a_corner;
uniform vec4 u_rect;   // (x0,y0, x1,y1) in NDC: (left,bottom)→(right,top)
out vec2 v_uv;
void main() {
    vec2 p = mix(u_rect.xy, u_rect.zw, a_corner);
    gl_Position = vec4(p, 0.0, 1.0);
    v_uv = vec2(a_corner.x, 1.0 - a_corner.y); // texture origin top-left
}
"#;

const OVERLAY_FS: &str = r#"#version 300 es
precision highp float;
in vec2 v_uv;
uniform sampler2D u_tex;
uniform float u_alpha;
uniform int u_mode; // OverlayCmd.blit_mode: 0=Normal, 1=Greenscreen, 2=Chroma
out vec4 frag;
void main() {
    vec4 t = texture(u_tex, v_uv);
    if (u_mode == 1) {
        // Greenscreen (`.g`): key out green-dominant texels; soft edge kills halo.
        float greenness = t.g - max(t.r, t.b);
        float key = 1.0 - smoothstep(0.15, 0.35, greenness);
        frag = vec4(t.rgb, t.a * u_alpha * key);
    } else if (u_mode == 2) {
        // Chroma (`.c`, leo3d's glas.c lens): opacity tracks CHROMA (max-min) —
        // vivid blue tint → the transparent window (world shows through); the black
        // vignette, the white highlight AND the dark rim are low-chroma so they stay
        // opaque, giving a smooth rim instead of a jagged one.
        float mx = max(t.r, max(t.g, t.b));
        float mn = min(t.r, min(t.g, t.b));
        frag = vec4(t.rgb, 1.0 - (mx - mn));
    } else {
        // Normal: straight alpha (a `.s`/`.a` mask, the corner matte, or an opaque
        // bitmap) scaled by the overlay's blend.
        frag = vec4(t.rgb, t.a * u_alpha);
    }
}
"#;

/// Grace period (in stage draws) a Groove scene keeps compositing after its last
/// `submit_frame`. Continuously-driven scenes reset the counter every frame, so
/// this only bounds how long a scene the game has NAVIGATED AWAY FROM lingers
/// before it fades out (e.g. BioBoxing's title arena when you open Instructions).
const STALE_DRAWS: i32 = 125;

struct OverlayShader {
    program: WebGlProgram,
    u_rect: Option<WebGlUniformLocation>,
    u_tex: Option<WebGlUniformLocation>,
    u_alpha: Option<WebGlUniformLocation>,
    u_mode: Option<WebGlUniformLocation>,
    vao: web_sys::WebGlVertexArrayObject,
}

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
    u_has_vcolor: Option<WebGlUniformLocation>,
    u_alpha: Option<WebGlUniformLocation>,
}

/// One GL-uploaded batch: its texture name (empty = untextured) and buffers.
struct GlBatch {
    tex_name: String,
    mesh: Mesh3dBuffers,
}

/// GL-side caches + shader. Lazily initialized.
pub struct XtraSceneRenderer {
    shader: Option<ShaderProgram>,
    overlay: Option<OverlayShader>,
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
        XtraSceneRenderer {
            shader: None,
            overlay: None,
            meshes: HashMap::new(),
            textures: HashMap::new(),
        }
    }

    /// Lazily build the 2D overlay shader + its unit-quad VAO.
    fn ensure_overlay(&mut self, context: &WebGL2Context) -> bool {
        if self.overlay.is_some() {
            return true;
        }
        let gl = context.gl();
        let vs = match context.compile_shader(WebGl2RenderingContext::VERTEX_SHADER, OVERLAY_VS) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let fs = match context.compile_shader(WebGl2RenderingContext::FRAGMENT_SHADER, OVERLAY_FS) {
            Ok(s) => s,
            Err(_) => return false,
        };
        let program = match context.link_program(&vs, &fs) {
            Ok(p) => p,
            Err(_) => return false,
        };
        let Some(vao) = gl.create_vertex_array() else { return false };
        let Some(vbo) = gl.create_buffer() else { return false };
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
        // Two triangles of the unit quad: corners (0,0)(1,0)(0,1)(1,1).
        let corners: [f32; 12] = [0.0, 0.0, 1.0, 0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0];
        unsafe {
            let view = js_sys::Float32Array::view(&corners);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &view,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);
        gl.bind_vertex_array(None);
        self.overlay = Some(OverlayShader {
            u_rect: gl.get_uniform_location(&program, "u_rect"),
            u_tex: gl.get_uniform_location(&program, "u_tex"),
            u_alpha: gl.get_uniform_location(&program, "u_alpha"),
            u_mode: gl.get_uniform_location(&program, "u_mode"),
            vao,
            program,
        });
        true
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
            u_has_vcolor: u("u_has_vcolor"),
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
        let scene_ids: Vec<i32> = with_store_mut(|store| {
            let mut ids = Vec::new();
            for (id, scene) in store.scenes.iter_mut() {
                // Composite the last submitted frame, and keep compositing it for a
                // short GRACE period after the game stops submitting. Two failure
                // modes to avoid:
                //  - The old strict gate (`last_submit_frame == current_frame`) cut
                //    the scene the instant the movie changed frame → hard-cut
                //    transitions, and it blacked out a menu that submits once then
                //    holds while the movie plays on.
                //  - Persisting FOREVER left a stale scene (e.g. BioBoxing's title
                //    arena) composited over a later Director-only screen
                //    (Instructions), hiding it.
                // The grace (draws_since_submit) smooths transitions yet lets a scene
                // the game has navigated away from fade out. A continuously-driven
                // scene resets the counter every submit and always renders.
                if scene.frame.is_none() {
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
        // Bail only when there's nothing to composite at all. A menu / title
        // screen is 2D overlays over a background with NO 3D models, so
        // `frame.draws` is empty while `frame.overlays` is not — returning here
        // would skip the overlay pass (draw_overlays, below) and leave the menu
        // blank. The 3D mesh/texture/draw loops that follow all iterate
        // `frame.draws`, so they naturally no-op when it's empty.
        if frame.draws.is_empty() && frame.overlays.is_empty() {
            return;
        }

        // (1) Ensure GL meshes for every mesh referenced this frame.
        //
        // Both this and the texture pass below key off the UNIQUE mesh ids, not
        // the draw list. A shape emits one DrawCmd per (part, texture) batch, so
        // a many-part model has hundreds of draws all naming the same mesh — and
        // the texture pass walks that mesh's entire batch list. Doing either
        // per-draw is O(draws × batches): the Hey Arnold level (767 parts, 113
        // textures, ~800 batches) burned ~640k redundant String clones and
        // ensure_texture calls *per frame* on that one object.
        let mut mesh_ids: Vec<u32> = Vec::new();
        for d in &frame.draws {
            if !mesh_ids.contains(&d.mesh_id) {
                mesh_ids.push(d.mesh_id);
            }
        }
        for &mesh_id in &mesh_ids {
            self.ensure_mesh(context, scene_id, mesh_id);
        }
        // (2) Ensure textures for every name referenced this frame: each unique
        // mesh's batch names, plus any per-object tex_override. Collected first
        // (self.meshes is borrowed here, ensure_texture needs &mut self).
        let mut tex_names: HashSet<String> = HashSet::new();
        for &mesh_id in &mesh_ids {
            if let Some((_, batches)) = self.meshes.get(&(scene_id, mesh_id)) {
                for b in batches {
                    if !b.tex_name.is_empty() && !tex_names.contains(b.tex_name.as_str()) {
                        tex_names.insert(b.tex_name.clone());
                    }
                }
            }
        }
        for d in &frame.draws {
            if let Some(name) = d.tex_override.as_deref().filter(|n| !n.is_empty()) {
                if !tex_names.contains(name) {
                    tex_names.insert(name.to_string());
                }
            }
        }
        for name in &tex_names {
            self.ensure_texture(context, player, scene_id, name, false);
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
        // Per-object depth priority (SetObjectDepth) uses polygon offset so coplanar
        // surfaces (a TV screen + its scanline overlay) don't z-fight/flicker.
        gl.enable(WebGl2RenderingContext::POLYGON_OFFSET_FILL);
        // Back-face culling is per-draw (bfculling); front faces are CCW.
        gl.front_face(WebGl2RenderingContext::CCW);

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
        // Large far plane: Groove worlds are big and some views (leo3d's fernglas
        // dolly-zoom) look at very distant features. A tighter far clips them —
        // the TV-screen moiré that prompted a tighter far was actually back-face
        // culling, not depth precision, so keep the original generous range.
        //
        // Groove's `Perspective` is the HORIZONTAL field of view, not the
        // vertical one. The engine builds a pinhole frustum from a single focal
        // length — `tan(fov_x/2) = (width/2)/focal`, `tan(fov_y/2) =
        // (height/2)/focal` — so the vertical FOV follows the width:height ratio.
        // Feeding `fov` to a vertical-FOV projection made a wide window far too
        // tall a view (Dora Soccer's follow-cam looked top-down/zoomed-out
        // instead of over-the-shoulder). Convert the horizontal FOV to the
        // equivalent vertical one for this aspect: fov_y = 2·atan(tan(fov_x/2)/aspect).
        let fov_x = frame.camera.fov.to_radians().max(0.1);
        let fov_y = 2.0 * ((fov_x * 0.5).tan() / aspect.max(1e-3)).atan();
        let proj = perspective(fov_y, aspect, 1.0, 200_000.0);
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
            // The world background (SetWorldBackground) never occludes and is never
            // occluded: it writes no depth, so every real surface composites over
            // it regardless of how near the skydome's own geometry actually is.
            // The plugin already re-centred it on the camera and submits it first.
            gl.depth_mask(!d.background && alpha >= 0.999);
            gl.uniform1i(shader.u_tex.as_ref(), 0);
            // Back-face culling per the shape's bfculling flag. Groove models bake
            // double-sided coplanar faces (screen quads wound both ways); culling
            // the back-facing copy stops the two from z-fighting into a moiré.
            if d.cull {
                gl.enable(WebGl2RenderingContext::CULL_FACE);
                gl.cull_face(WebGl2RenderingContext::BACK);
            } else {
                gl.disable(WebGl2RenderingContext::CULL_FACE);
            }
            // SetObjectDepth priority. A fixed value >= 1 is a FRONT LAYER: Groove
            // composites it on top of the scene, so we skip the depth test (else a
            // near-coplanar front object — the 3D logo box over the truss — z-fights
            // into a moiré/hatch). Auto (-1/0) and draw-behind (-2) keep the normal
            // depth test, with a polygon offset to separate coplanar surfaces.
            if d.background || d.depth >= 1 {
                gl.depth_func(WebGl2RenderingContext::ALWAYS);
                gl.polygon_offset(0.0, 0.0);
            } else {
                gl.depth_func(WebGl2RenderingContext::LEQUAL);
                let poff = if d.depth == -2 { 2.0 } else { 0.0 };
                gl.polygon_offset(poff, poff);
            }

            // Select this cmd's batch by index; `batch < 0` means "all batches
            // of the mesh" (deformed objects submit one cmd for the whole mesh).
            // Indexing, not scan-and-skip: a shape's per-batch draws each walked
            // the full batch list, making the pass O(draws × batches) per frame.
            let selected: &[GlBatch] = if d.batch >= 0 {
                match batches.get(d.batch as usize) {
                    Some(b) => std::slice::from_ref(b),
                    None => &[],
                }
            } else {
                batches.as_slice()
            };
            for batch in selected {
                let tex_name = d
                    .tex_override
                    .as_deref()
                    .filter(|n| !n.is_empty())
                    .unwrap_or(batch.tex_name.as_str());
                // Groove `.add` textures are drawn with ADDITIVE blending (engine
                // blend mode 2 = SRC_ALPHA, ONE): black areas add nothing (read as
                // transparent), bright areas glow over the scene. Everything else
                // uses the standard alpha blend.
                let additive = tex_name.len() >= 4
                    && tex_name[tex_name.len() - 4..].eq_ignore_ascii_case(".add");
                if additive {
                    gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE);
                    gl.depth_mask(false);
                } else {
                    gl.blend_func(
                        WebGl2RenderingContext::SRC_ALPHA,
                        WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
                    );
                    // Must repeat the background test the outer draw made: a
                    // background never writes depth. Setting it from `alpha` alone
                    // let the skydome — drawn with depth_func(ALWAYS) at the camera
                    // — stamp near depth over the whole screen, so every farther
                    // surface (the street; the houses, intermittently) failed LEQUAL.
                    gl.depth_mask(!d.background && alpha >= 0.999);
                }
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
                // Batches with baked material colors render unlit (self-illuminated).
                gl.uniform1i(shader.u_has_vcolor.as_ref(), batch.mesh.has_vertex_colors as i32);
                batch.mesh.bind(gl);
                batch.mesh.draw(gl);
                batch.mesh.unbind(gl);
            }
        }
        gl.depth_mask(true);
        gl.polygon_offset(0.0, 0.0);
        gl.disable(WebGl2RenderingContext::POLYGON_OFFSET_FILL);
        gl.disable(WebGl2RenderingContext::CULL_FACE);
        gl.disable(WebGl2RenderingContext::BLEND);
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::SCISSOR_TEST);
        gl.viewport(0, 0, fb_w, fb_h);

        // 2D bitmap overlays over the full stage, composited after the 3D scene.
        self.draw_overlays(context, player, scene_id, &frame.overlays, viewport_w, viewport_h);
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
            let mut out: Vec<(String, Vec<[f32; 3]>, Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 4]>)> =
                Vec::new();
            if let Some(m) = store.scenes.get(&scene_id).and_then(|s| s.meshes.get(&mesh_id)) {
                for b in &m.data.batches {
                    out.push((
                        b.tex_name.clone(),
                        chunk3(&b.positions),
                        chunk3(&b.normals),
                        chunk2(&b.uvs),
                        chunk4(&b.colors),
                    ));
                }
            }
            out
        });
        let mut gl_batches = Vec::new();
        for (tex_name, positions, normals, uvs, colors) in batches {
            let n_tris = positions.len() / 3;
            let faces: Vec<[u32; 3]> =
                (0..n_tris).map(|i| [(i * 3) as u32, (i * 3 + 1) as u32, (i * 3 + 2) as u32]).collect();
            let uvs_opt = if uvs.len() == positions.len() { Some(uvs.as_slice()) } else { None };
            let colors_opt = if colors.len() == positions.len() { Some(colors.as_slice()) } else { None };
            if let Ok(mesh) = Mesh3dBuffers::new_full(
                context, &positions, &normals, uvs_opt, None, &faces, None, None, colors_opt,
            ) {
                gl_batches.push(GlBatch { tex_name, mesh });
            }
        }
        self.meshes.insert((scene_id, mesh_id), (store_gen, gl_batches));
    }

    /// Ensure a GL texture for `name` in `scene_id`. Prefers a plugin-uploaded
    /// RGBA texture (tracked by store generation); otherwise resolves `name` to a
    /// movie bitmap cast member and uploads it once.
    /// `for_overlay`: a 2D overlay sprite rather than a 3D model texture. An
    /// untagged sprite (the character glyphs) is still a cut-out and needs its
    /// key colour removed, whereas an untagged 3D texture is Solid and must be
    /// left opaque — keying a road or wall would punch holes in it.
    fn ensure_texture(
        &mut self,
        context: &WebGL2Context,
        player: &DirPlayer,
        scene_id: i32,
        name: &str,
        for_overlay: bool,
    ) {
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
        let tex = upload_named_texture(context, player, name, for_overlay);
        self.textures.insert(key, (CAST_TEX_GEN, tex));
    }

    /// Native pixel size of an overlay's texture — a plugin-uploaded sprite
    /// (store) or a movie bitmap cast member resolved by name.
    fn overlay_texture_size(&self, player: &DirPlayer, scene_id: i32, name: &str) -> Option<(i32, i32)> {
        let store_size = with_store_mut(|store| {
            store
                .scenes
                .get(&scene_id)
                .and_then(|s| s.textures.get(name))
                .map(|t| (t.w as i32, t.h as i32))
        });
        if let Some((w, h)) = store_size {
            if w > 0 && h > 0 {
                return Some((w, h));
            }
        }
        let mref = player.movie.cast_manager.find_member_ref_by_name(name)?;
        let member = player.movie.cast_manager.find_member_by_ref(&mref)?;
        let bref = match &member.member_type {
            CastMemberType::Bitmap(b) => b.image_ref,
            _ => return None,
        };
        let bitmap = player.bitmap_manager.get_bitmap(bref)?;
        Some((bitmap.width as i32, bitmap.height as i32))
    }

    /// Composite the frame's 2D bitmap overlays over the full stage (Groove
    /// `AddOverlay`). Center-anchored at `loc` in stage px; blend → alpha.
    fn draw_overlays(
        &mut self,
        context: &WebGL2Context,
        player: &DirPlayer,
        scene_id: i32,
        overlays: &[xtra_sdk::scene3d::OverlayCmd],
        viewport_w: i32,
        viewport_h: i32,
    ) {
        if overlays.is_empty() || !self.ensure_overlay(context) {
            return;
        }
        // Resolve every overlay's texture + native size first (mutates self).
        for ov in overlays {
            if !ov.tex_name.is_empty() {
                self.ensure_texture(context, player, scene_id, &ov.tex_name, true);
            }
        }
        let gl = context.gl();
        let fb_w = gl.drawing_buffer_width();
        let fb_h = gl.drawing_buffer_height();
        gl.viewport(0, 0, fb_w, fb_h);
        gl.disable(WebGl2RenderingContext::DEPTH_TEST);
        gl.disable(WebGl2RenderingContext::SCISSOR_TEST);
        gl.enable(WebGl2RenderingContext::BLEND);
        gl.blend_func(WebGl2RenderingContext::SRC_ALPHA, WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA);
        let ov_shader = self.overlay.as_ref().unwrap();
        gl.use_program(Some(&ov_shader.program));
        gl.bind_vertex_array(Some(&ov_shader.vao));
        gl.uniform1i(ov_shader.u_tex.as_ref(), 0);
        for ov in overlays {
            let tex = self
                .textures
                .get(&(scene_id, ov.tex_name.clone()))
                .and_then(|(_, t)| t.as_ref());
            let Some(tex) = tex else { continue };
            let (nw, nh) = self.overlay_texture_size(player, scene_id, &ov.tex_name).unwrap_or((0, 0));
            let w = if ov.size[0] > 0 { ov.size[0] } else { nw };
            let h = if ov.size[1] > 0 { ov.size[1] } else { nh };
            if w <= 0 || h <= 0 {
                continue;
            }
            // Reg-point-anchored stage-pixel rect → NDC (y flipped, top-left
            // origin). The plugin places the overlay so its `regpoint` lands on
            // `loc` (top-left = loc − regpoint); an un-set reg point arrives as
            // `size/2`, reproducing the old centre anchor. This is what lets
            // Dora Soccer's `huddy` bar (reg point at its bottom-centre) sit in
            // the bottom tray instead of being pushed half off-screen.
            let (lx, ly) = (
                ov.loc[0] as f32 - ov.regpoint[0] as f32,
                ov.loc[1] as f32 - ov.regpoint[1] as f32,
            );
            let (left, right) = (lx, lx + w as f32);
            let (top, bottom) = (ly, ly + h as f32);
            let vw = viewport_w.max(1) as f32;
            let vh = viewport_h.max(1) as f32;
            let ndc_x = |x: f32| x / vw * 2.0 - 1.0;
            let ndc_y = |y: f32| 1.0 - y / vh * 2.0;
            // Transparency mode comes from the PLUGIN (OverlayCmd.blit_mode):
            // 0 = Normal alpha, 1 = Greenscreen (`.g`), 2 = Chroma (`.c`). The host
            // no longer sniffs `tex_name` — the plugin owns the extension decision.
            // All modes use standard SRC_ALPHA over-blending.
            gl.blend_func(
                WebGl2RenderingContext::SRC_ALPHA,
                WebGl2RenderingContext::ONE_MINUS_SRC_ALPHA,
            );
            gl.uniform1i(ov_shader.u_mode.as_ref(), ov.blit_mode as i32);
            // u_rect = (left,bottom)→(right,top) so corner (0,0)=bottom-left.
            gl.uniform4f(ov_shader.u_rect.as_ref(), ndc_x(left), ndc_y(bottom), ndc_x(right), ndc_y(top));
            gl.uniform1f(ov_shader.u_alpha.as_ref(), (ov.blend / 100.0).clamp(0.0, 1.0));
            gl.active_texture(WebGl2RenderingContext::TEXTURE0);
            gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(tex));
            gl.draw_arrays(WebGl2RenderingContext::TRIANGLES, 0, 6);
        }
        gl.bind_vertex_array(None);
        gl.disable(WebGl2RenderingContext::BLEND);
    }
}

fn chunk3(v: &[f32]) -> Vec<[f32; 3]> {
    v.chunks_exact(3).map(|c| [c[0], c[1], c[2]]).collect()
}
fn chunk2(v: &[f32]) -> Vec<[f32; 2]> {
    v.chunks_exact(2).map(|c| [c[0], c[1]]).collect()
}
fn chunk4(v: &[f32]) -> Vec<[f32; 4]> {
    v.chunks_exact(4).map(|c| [c[0], c[1], c[2], c[3]]).collect()
}

fn upload_rgba(context: &WebGL2Context, w: u32, h: u32, rgba: &[u8]) -> Option<WebGlTexture> {
    if w == 0 || h == 0 || rgba.len() != (w * h * 4) as usize {
        return None;
    }
    let tex = context.create_texture().ok()?;
    context.upload_texture_rgba(&tex, w, h, rgba).ok()?;
    // Groove 3D surfaces tile textures via UVs > 1 (e.g. the stands repeat a
    // spectator strip 4× across each row). The shared uploader sets CLAMP_TO_EDGE,
    // which would fill only the first tile — override to REPEAT. WebGL2 allows
    // REPEAT on NPOT textures; for 0..1 UVs (overlays) it's identical to clamp.
    let gl = context.gl();
    gl.bind_texture(WebGl2RenderingContext::TEXTURE_2D, Some(&tex));
    gl.tex_parameteri(
        WebGl2RenderingContext::TEXTURE_2D,
        WebGl2RenderingContext::TEXTURE_WRAP_S,
        WebGl2RenderingContext::REPEAT as i32,
    );
    gl.tex_parameteri(
        WebGl2RenderingContext::TEXTURE_2D,
        WebGl2RenderingContext::TEXTURE_WRAP_T,
        WebGl2RenderingContext::REPEAT as i32,
    );
    // Trilinear + mipmaps. The shared uploader sets NEAREST, which aliases badly
    // when high-frequency 3D textures (the stands' net/grid, distant screens) are
    // minified — producing a shimmering moiré that flickers as the camera orbits.
    // Mipmapped LINEAR resolves it (WebGL2 allows mipmaps on NPOT textures).
    gl.generate_mipmap(WebGl2RenderingContext::TEXTURE_2D);
    gl.tex_parameteri(
        WebGl2RenderingContext::TEXTURE_2D,
        WebGl2RenderingContext::TEXTURE_MIN_FILTER,
        WebGl2RenderingContext::LINEAR_MIPMAP_LINEAR as i32,
    );
    gl.tex_parameteri(
        WebGl2RenderingContext::TEXTURE_2D,
        WebGl2RenderingContext::TEXTURE_MAG_FILTER,
        WebGl2RenderingContext::LINEAR as i32,
    );
    // Anisotropic filtering. The TV screens sit on arena walls viewed at steep
    // angles, where plain trilinear mipmapping still aliases the fine screen/LED
    // pattern into a moiré "net". EXT_texture_filter_anisotropic resolves it.
    // TEXTURE_MAX_ANISOTROPY_EXT = 0x84FE.
    if matches!(gl.get_extension("EXT_texture_filter_anisotropic"), Ok(Some(_))) {
        gl.tex_parameterf(WebGl2RenderingContext::TEXTURE_2D, 0x84FE, 16.0);
    }
    Some(tex)
}

/// Resolve a movie bitmap cast member by name and expand it to opaque RGBA
/// (native-depth `bitmap.data` — 8-bit indexed / 16 / 32 — via the cast palette).
fn resolve_bitmap_rgba(player: &DirPlayer, name: &str) -> Option<(usize, usize, Vec<u8>)> {
    let mref = player.movie.cast_manager.find_member_ref_by_name(name)?;
    let member = player.movie.cast_manager.find_member_by_ref(&mref)?;
    let bref = match &member.member_type {
        CastMemberType::Bitmap(b) => b.image_ref,
        _ => return None,
    };
    let bitmap = player.bitmap_manager.get_bitmap(bref)?;
    let palettes = player.movie.cast_manager.palettes();
    let rgba = super::WebGL2Renderer::bitmap_to_rgba(bitmap, &palettes, 0, None, None, None, false);
    Some((bitmap.width as usize, bitmap.height as usize, rgba))
}

/// Resolve `name` to a movie bitmap cast member and upload it as a GL texture.
///
/// Groove sprites carry transparency as a SEPARATE companion bitmap: `<base>.s`
/// is the 32-bit color image and `<base>.a` is an 8-bit **grayscale alpha mask**
/// (white = opaque, black = transparent — the sprite silhouette). The two are
/// authored as a pair. So load the `.s` color, then load the `.a` mask and use
/// its luminance as the alpha channel. Only when there is no `.a` companion do we
/// fall back to the corner-color matte (plain sprites like the character glyphs,
/// which have no mask).
fn upload_named_texture(
    context: &WebGL2Context,
    player: &DirPlayer,
    name: &str,
    for_overlay: bool,
) -> Option<WebGlTexture> {
    let (w, h, mut rgba) = resolve_bitmap_rgba(player, name)?;
    // The asset-name extension IS the Groove BlitMode, and dirplayer loads Groove
    // bitmaps OPAQUE, so reconstruct the transparency it implies:
    //
    //   .s  Solid colour image, paired with a `.a` greyscale alpha mask
    //   .t  Transparent  → colour-key the background
    //   .g  Greenscreen  → key out green-dominant texels
    //   .a  Alpha        → the bitmap already carries its own alpha
    //   none/other       → Solid: key NOTHING
    //
    // Keying unconditionally (the old behaviour) is only invisible while the 3D
    // shader ignores texel alpha; once it honours it, a matte on an untagged
    // texture punches its border colour out of solid geometry (roads, walls).
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase());
    match ext.as_deref() {
        Some("s") => {
            // Companion mask: `foo.s` -> `foo.a`.
            let base = &name[..name.len() - 2];
            let mut used_mask = false;
            if let Some((aw, ah, amask)) = resolve_bitmap_rgba(player, &format!("{}.a", base)) {
                if aw == w && ah == h {
                    // Grayscale mask → R == G == B; take R as the alpha value.
                    for i in 0..(w * h) {
                        rgba[i * 4 + 3] = amask[i * 4];
                    }
                    used_mask = true;
                }
            }
            // A `.s` with no authored mask is a plain cut-out sprite.
            if !used_mask {
                key_corner_transparent(&mut rgba, w, h);
            }
        }
        Some("t") => key_corner_transparent(&mut rgba, w, h),
        Some("g") => greenscreen_transparent(&mut rgba, w, h),
        // Untagged: Solid for a 3D texture, but an overlay sprite (the character
        // glyphs) is still a cut-out keyed on its background colour.
        _ if for_overlay => key_corner_transparent(&mut rgba, w, h),
        _ => {}
    }
    upload_rgba(context, w as u32, h as u32, &rgba)
}

/// Groove Greenscreen blit mode (`.g`): key out green-dominant texels. Mirrors
/// the overlay shader's rule (`greenness = g - max(r, b)`) so a texture and an
/// overlay of the same art cut out identically.
fn greenscreen_transparent(rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return;
    }
    for i in 0..(w * h) {
        let p = i * 4;
        let (r, g, b) = (rgba[p] as i32, rgba[p + 1] as i32, rgba[p + 2] as i32);
        if g - r.max(b) > 40 {
            rgba[p + 3] = 0;
        }
    }
}

/// Color-key matte for plain Groove overlay sprites that have NO `.a` alpha
/// companion (the character glyphs). The key color is the most common color
/// along the border (the sprite's background). EVERY pixel of that color is
/// keyed to alpha 0 — not just the edge-connected region — so the enclosed
/// holes of letters like O/G also become transparent and reveal the button
/// behind, instead of showing the key color. Glyphs are single-color letters on
/// a flat key background, so keying all key-color pixels is safe. Sprites that
/// carry a real silhouette (title/buttons) use their `.a` mask and never reach
/// this path.
fn key_corner_transparent(rgba: &mut [u8], w: usize, h: usize) {
    if w == 0 || h == 0 || rgba.len() < w * h * 4 {
        return;
    }
    // The key is the colour of the TOP-LEFT pixel — Director's transparent-ink
    // rule (an ink with no explicit bgColor keys on src (0,0)). Every pixel of
    // that colour is keyed, not just the edge-connected region, so the enclosed
    // holes of glyphs like O and G become transparent too.
    //
    // NOT the dominant border colour: an authored key sits at (0,0) but need not
    // dominate the border. `busf.t` is keyed green (0,255,0) at (0,0) while its
    // border is mostly the blue window frame — matting the border cut the frame
    // and left the windows green.
    let key = [rgba[0], rgba[1], rgba[2]];
    for i in 0..(w * h) {
        let p = i * 4;
        if rgba[p] == key[0] && rgba[p + 1] == key[1] && rgba[p + 2] == key[2] {
            rgba[p + 3] = 0;
        }
    }
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
