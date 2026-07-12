// SPDX-License-Identifier: GPL-3.0-only
//
//! Host-side store for the external Xtra 3D scene API (`HostOp::Scene3d*`).
//!
//! An external plugin (e.g. a Groove-Xtra wasm) owns all world simulation and,
//! per step, pushes *retained* draw data at the host through the `scene3d_*`
//! host services. Those calls arrive mid-Lingo-execution, with no GL context in
//! scope, so they cannot touch WebGL directly. This module is the buffer
//! between them and the renderer: the [`host_call_dispatch`] arms in
//! `external.rs` write here, and [`XtraSceneRenderer`] in
//! `rendering_gpu::webgl2::xtra_scene` reads here during the normal draw pass,
//! uploading GL buffers/textures lazily and compositing the latest frame.
//!
//! This mirrors the built-in Groove split (`GrooveXtraManager` state ←
//! `GrooveSceneRenderer`), but is engine-agnostic: any plugin driving the
//! `scene3d` ops shares it.

use std::collections::HashMap;

use xtra_sdk::scene3d::{FrameData, MeshData};

/// One uploaded mesh (the batch set for a shape or a deformed object). `generation`
/// bumps on every re-upload so the renderer knows to rebuild its GL buffers.
pub struct SceneMesh {
    pub data: MeshData,
    pub generation: u64,
}

/// One CPU-composed texture uploaded by the plugin (`Scene3dUploadTexture`).
/// Cast-member textures are *not* stored here — the renderer resolves those by
/// name against movie bitmaps. `generation` bumps on re-upload.
pub struct UploadedTexture {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    pub generation: u64,
}

/// One scene: its meshes, plugin-uploaded textures, and the latest frame to
/// composite, plus the staleness bookkeeping the renderer uses to stop
/// painting once the game stops stepping.
pub struct Scene {
    pub meshes: HashMap<u32, SceneMesh>,
    pub textures: HashMap<String, UploadedTexture>,
    pub frame: Option<FrameData>,
    /// The movie frame in effect when the latest frame was submitted. The
    /// renderer only composites while this equals the current movie frame, so
    /// stale 3D stops painting over later 2D/Flash content (mirrors the
    /// Groove `last_active_frame` gate).
    pub last_submit_frame: i32,
    /// Draws since the last submit; the renderer bumps this and stops after a
    /// small tolerance so a redraw not paired with a step doesn't linger.
    pub draws_since_submit: i32,
}

impl Scene {
    fn new() -> Self {
        Scene {
            meshes: HashMap::new(),
            textures: HashMap::new(),
            frame: None,
            last_submit_frame: -1,
            draws_since_submit: 0,
        }
    }
}

/// The whole store: tag→id map (for idempotent `create`) plus the live scenes.
#[derive(Default)]
pub struct Scene3dStore {
    tags: HashMap<String, i32>,
    pub scenes: HashMap<i32, Scene>,
    next_id: i32,
}

impl Scene3dStore {
    pub fn new() -> Self {
        Scene3dStore { tags: HashMap::new(), scenes: HashMap::new(), next_id: 1 }
    }

    /// Idempotent per tag: returns the existing scene id or mints a new one.
    pub fn create(&mut self, tag: &str) -> i32 {
        if let Some(id) = self.tags.get(tag) {
            return *id;
        }
        let id = self.next_id;
        self.next_id += 1;
        self.tags.insert(tag.to_string(), id);
        self.scenes.insert(id, Scene::new());
        id
    }

    pub fn upload_mesh(&mut self, scene_id: i32, mesh_id: u32, data: MeshData) {
        if let Some(scene) = self.scenes.get_mut(&scene_id) {
            let generation = scene.meshes.get(&mesh_id).map(|m| m.generation + 1).unwrap_or(0);
            scene.meshes.insert(mesh_id, SceneMesh { data, generation });
        }
    }

    pub fn drop_mesh(&mut self, scene_id: i32, mesh_id: u32) {
        if let Some(scene) = self.scenes.get_mut(&scene_id) {
            scene.meshes.remove(&mesh_id);
        }
    }

    pub fn upload_texture(&mut self, scene_id: i32, name: &str, w: u32, h: u32, rgba: Vec<u8>) {
        if let Some(scene) = self.scenes.get_mut(&scene_id) {
            let generation = scene.textures.get(name).map(|t| t.generation + 1).unwrap_or(0);
            scene.textures.insert(name.to_string(), UploadedTexture { w, h, rgba, generation });
        }
    }

    /// Store the latest frame and stamp the movie frame it was submitted on.
    pub fn submit_frame(&mut self, scene_id: i32, frame: FrameData, movie_frame: i32) {
        if let Some(scene) = self.scenes.get_mut(&scene_id) {
            scene.frame = Some(frame);
            scene.last_submit_frame = movie_frame;
            scene.draws_since_submit = 0;
        }
    }

    pub fn destroy(&mut self, scene_id: i32) {
        self.scenes.remove(&scene_id);
        self.tags.retain(|_, id| *id != scene_id);
    }
}

/// The global store. WASM is single-threaded, so `static mut` is sound — the
/// same pattern the built-in Groove manager uses. Populated by
/// `external::host_call_dispatch`, read by the webgl2 `XtraSceneRenderer`.
pub static mut XTRA_SCENE_STORE: Option<Scene3dStore> = None;

/// Run `f` against the store, initializing it on first use.
pub fn with_store_mut<R>(f: impl FnOnce(&mut Scene3dStore) -> R) -> R {
    unsafe {
        let ptr = &raw mut XTRA_SCENE_STORE;
        let store = (*ptr).get_or_insert_with(Scene3dStore::new);
        f(store)
    }
}

/// Drop every scene. Called from `DirPlayer::reset` so a plugin's scenes from a
/// previous movie don't linger across loads (their GL resources are freed the
/// next time the renderer sees the scene is gone).
pub fn clear_all() {
    with_store_mut(|s| {
        s.scenes.clear();
        s.tags.clear();
    });
}
