//! Rendering module for DirPlayer
//!
//! This module provides additional rendering backends for the Director emulator.
//! The main Canvas 2D renderer is still in the parent rendering.rs file.
//! This module adds:
//! - WebGL2 (GPU-accelerated, optional) - work in progress

pub mod webgl2;

use wasm_bindgen::JsCast;
use web_sys::HtmlCanvasElement;

use crate::player::DirPlayer;

/// Renderer backend selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RendererBackend {
    /// Canvas 2D software rendering (default, always available)
    Canvas2D,
    /// WebGL2 hardware-accelerated rendering (optional)
    WebGL2,
}

impl Default for RendererBackend {
    fn default() -> Self {
        RendererBackend::Canvas2D
    }
}

/// Common trait for all rendering backends
pub trait Renderer {
    /// Draw the main stage frame
    fn draw_frame(&mut self, player: &mut DirPlayer);

    /// Draw the preview frame (member preview)
    fn draw_preview_frame(&mut self, player: &mut DirPlayer);

    /// Set the main canvas size
    fn set_size(&mut self, width: u32, height: u32);

    /// Get the current canvas size
    fn size(&self) -> (u32, u32);

    /// Get the backend name for debugging
    fn backend_name(&self) -> &'static str;

    /// Get the main canvas element
    fn canvas(&self) -> &HtmlCanvasElement;

    /// Set the preview member reference
    fn set_preview_member_ref(&mut self, member_ref: Option<crate::player::cast_lib::CastMemberRef>);

    /// Set the preview container element
    fn set_preview_container_element(&mut self, container_element: Option<web_sys::HtmlElement>);
}

/// Check if WebGL2 is supported in this browser
pub fn is_webgl2_supported() -> bool {
    if let Some(window) = web_sys::window() {
        if let Some(document) = window.document() {
            if let Ok(canvas) = document.create_element("canvas") {
                if let Ok(canvas) = canvas.dyn_into::<HtmlCanvasElement>() {
                    return canvas.get_context("webgl2").ok().flatten().is_some();
                }
            }
        }
    }
    false
}

/// A dynamic renderer that can switch between Canvas2D and WebGL2 backends
pub enum DynamicRenderer {
    Canvas2D(crate::rendering::PlayerCanvasRenderer),
    WebGL2(webgl2::WebGL2Renderer),
}

impl DynamicRenderer {
    /// Get the current backend type
    pub fn backend(&self) -> RendererBackend {
        match self {
            DynamicRenderer::Canvas2D(_) => RendererBackend::Canvas2D,
            DynamicRenderer::WebGL2(_) => RendererBackend::WebGL2,
        }
    }

    /// Get the Canvas2D renderer (for Canvas2D-specific operations)
    pub fn as_canvas2d(&self) -> Option<&crate::rendering::PlayerCanvasRenderer> {
        match self {
            DynamicRenderer::Canvas2D(r) => Some(r),
            DynamicRenderer::WebGL2(_) => None,
        }
    }

    /// Get the Canvas2D renderer mutably
    pub fn as_canvas2d_mut(&mut self) -> Option<&mut crate::rendering::PlayerCanvasRenderer> {
        match self {
            DynamicRenderer::Canvas2D(r) => Some(r),
            DynamicRenderer::WebGL2(_) => None,
        }
    }

    /// Get the WebGL2 renderer (for WebGL2-specific operations)
    pub fn as_webgl2(&self) -> Option<&webgl2::WebGL2Renderer> {
        match self {
            DynamicRenderer::Canvas2D(_) => None,
            DynamicRenderer::WebGL2(r) => Some(r),
        }
    }

    /// Get the WebGL2 renderer mutably
    pub fn as_webgl2_mut(&mut self) -> Option<&mut webgl2::WebGL2Renderer> {
        match self {
            DynamicRenderer::Canvas2D(_) => None,
            DynamicRenderer::WebGL2(r) => Some(r),
        }
    }
}

impl Renderer for DynamicRenderer {
    fn draw_frame(&mut self, player: &mut DirPlayer) {
        match self {
            DynamicRenderer::Canvas2D(r) => r.draw_frame(player),
            DynamicRenderer::WebGL2(r) => r.draw_frame(player),
        }
    }

    fn draw_preview_frame(&mut self, player: &mut DirPlayer) {
        match self {
            DynamicRenderer::Canvas2D(r) => r.draw_preview_frame(player),
            DynamicRenderer::WebGL2(r) => r.draw_preview_frame(player),
        }
    }

    fn set_size(&mut self, width: u32, height: u32) {
        match self {
            DynamicRenderer::Canvas2D(r) => r.set_size(width, height),
            DynamicRenderer::WebGL2(r) => r.set_size(width, height),
        }
    }

    fn size(&self) -> (u32, u32) {
        match self {
            DynamicRenderer::Canvas2D(r) => r.size(),
            DynamicRenderer::WebGL2(r) => r.size(),
        }
    }

    fn backend_name(&self) -> &'static str {
        match self {
            DynamicRenderer::Canvas2D(r) => r.backend_name(),
            DynamicRenderer::WebGL2(r) => r.backend_name(),
        }
    }

    fn canvas(&self) -> &HtmlCanvasElement {
        match self {
            DynamicRenderer::Canvas2D(r) => r.canvas(),
            DynamicRenderer::WebGL2(r) => r.canvas(),
        }
    }

    fn set_preview_member_ref(&mut self, member_ref: Option<crate::player::cast_lib::CastMemberRef>) {
        match self {
            DynamicRenderer::Canvas2D(r) => r.preview_member_ref = member_ref,
            DynamicRenderer::WebGL2(r) => r.set_preview_member_ref(member_ref),
        }
    }

    fn set_preview_container_element(&mut self, container_element: Option<web_sys::HtmlElement>) {
        match self {
            DynamicRenderer::Canvas2D(r) => r.set_preview_container_element(container_element),
            DynamicRenderer::WebGL2(r) => r.set_preview_container_element(container_element),
        }
    }
}
