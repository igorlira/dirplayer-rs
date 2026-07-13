// SPDX-License-Identifier: GPL-3.0-only
//
//! Typed 3D scene frames crossing the host↔plugin boundary.
//!
//! The base [`Datum`](crate::Datum) wire only carries scalar/list values, which
//! is a poor fit for the dense f32 buffers a 3D engine plugin (e.g. the Groove
//! Xtra) needs to push at the host renderer. Rather than widen `Datum` with
//! geometry variants, these structs are postcard-encoded into a
//! [`Datum::ByteArray`](crate::Datum::ByteArray) — the SDK's binary channel — so
//! host and plugin share one source of truth and cannot drift.
//!
//! ## Split of responsibility
//!
//! - **Meshes** ([`MeshData`]) cross **once**, when the plugin creates a shape,
//!   and again only when that shape's geometry is edited (deform). They are
//!   non-indexed, flat-shaded, per-material triangle batches.
//! - **Frames** ([`FrameData`]) cross **every step** and carry only draw
//!   commands — a per-batch model matrix + material, *not* geometry. The plugin
//!   evaluates its own animation and ships the resulting matrices, so per-frame
//!   traffic stays small (a few KB of f32s even for a busy scene).
//!
//! See `docs/Groove-Xtra-Plugin-Plan.md` for the full rationale.

use alloc::{string::String, vec::Vec};
use serde::{Deserialize, Serialize};

/// One non-indexed, flat-shaded triangle batch of a single material.
///
/// `positions`/`normals` are `3 * vertex_count` long, `uvs` is
/// `2 * vertex_count`; vertices are consumed three-at-a-time as triangles
/// (index `0,1,2, 3,4,5, …`). `tex_name` is the host-resolved texture name
/// (a movie bitmap cast member, or a name uploaded via `Scene3dUploadTexture`);
/// empty means untextured.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshBatch {
    pub tex_name: String,
    pub positions: Vec<f32>,
    pub normals: Vec<f32>,
    pub uvs: Vec<f32>,
    /// Optional per-vertex color, `4 * vertex_count` (RGBA, 0..1) or empty. When
    /// non-empty the host renders this batch **unlit** (the color is the final
    /// fragment color, mirroring an engine that bakes material emission/diffuse
    /// into flat vertex colors — e.g. Groove's `glColorPointer`); empty keeps the
    /// batch on the lit/textured path.
    #[serde(default)]
    pub colors: Vec<f32>,
}

/// A complete uploadable mesh — the batch set for one shape or one deformed
/// object. Re-uploading under the same mesh id replaces the previous data.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct MeshData {
    pub batches: Vec<MeshBatch>,
}

/// Camera for a frame. `pos`/`look_at` are world-space; `fov` is the vertical
/// field of view in degrees. Up is +Z (the engine's world up).
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Camera {
    pub pos: [f32; 3],
    pub look_at: [f32; 3],
    pub fov: f32,
}

/// A single directional light. `dir` is the direction the light travels;
/// `color` is linear 0..1 RGB.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Light {
    pub dir: [f32; 3],
    pub color: [f32; 3],
}

/// One draw call: place batch(es) of an uploaded mesh with a model matrix and
/// material. `batch = -1` draws every batch of the mesh; `>= 0` selects one.
/// `model` is a column-major (GL order) 4×4. `tex_override`, when set, replaces
/// every batch's own texture name for this draw.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DrawCmd {
    pub mesh_id: u32,
    pub batch: i32,
    pub model: [f32; 16],
    pub color: [u8; 3],
    pub alpha: f32,
    pub tex_override: Option<String>,
    /// Groove `SetObjectDepth` priority: `-1` auto, `-2` draw-behind, `>= 0` fixed
    /// (higher = closer to camera). The host applies a matching polygon-offset so
    /// coplanar surfaces (e.g. a screen + its scanline overlay) don't z-fight.
    #[serde(default)]
    pub depth: i32,
    /// Back-face culling (the shape's `Atr2` bfculling flag). Groove models are
    /// authored with double-sided coplanar faces (e.g. a screen quad wound both
    /// ways); without culling the two opposite-wound triangles z-fight into a
    /// moiré. When true the host culls back faces (front = CCW).
    #[serde(default)]
    pub cull: bool,
}

/// A 2D screen-space bitmap overlay composited over (or under) the 3D scene
/// (Groove `AddOverlay`). Center-anchored at `loc` in stage pixels; `size` `[0,0]`
/// means use the texture's native size. `blend` is 0..100 (→ alpha). `channel`
/// is z-order: negative draws below the 3D scene, `>= 0` above it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OverlayCmd {
    pub tex_name: String,
    pub loc: [i32; 2],
    pub size: [i32; 2],
    pub blend: f32,
    pub channel: i32,
}

/// A full frame to composite. `render_rect` (stage pixels, l/t/r/b) bounds the
/// 3D view; `None` means the whole stage. `background`, when set, clears the
/// rect to that color (an opaque windowed view); `None` composites over the 2D
/// layer without a color clear.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FrameData {
    pub render_rect: Option<(i32, i32, i32, i32)>,
    pub background: Option<[u8; 3]>,
    pub camera: Camera,
    /// Ambient floor, 0..1 (already mapped from any engine-specific percent).
    pub ambient: f32,
    pub light: Option<Light>,
    pub draws: Vec<DrawCmd>,
    /// 2D bitmap overlays (Groove `AddOverlay`), composited in stage space after
    /// the 3D scene, ordered by `channel`.
    #[serde(default)]
    pub overlays: Vec<OverlayCmd>,
}

impl MeshData {
    /// Postcard-encode for transport inside a `Datum::ByteArray`.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }
    /// Decode a payload produced by [`MeshData::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}

impl FrameData {
    /// Postcard-encode for transport inside a `Datum::ByteArray`.
    pub fn to_bytes(&self) -> Vec<u8> {
        postcard::to_allocvec(self).unwrap_or_default()
    }
    /// Decode a payload produced by [`FrameData::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, postcard::Error> {
        postcard::from_bytes(bytes)
    }
}
