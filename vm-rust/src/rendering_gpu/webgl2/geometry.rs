//! Geometry buffers for WebGL2 sprite rendering
//!
//! This module provides the quad geometry (VAO/VBO) used to render sprites.
//! Each sprite is rendered as a textured quad with position/size/rotation
//! applied via uniforms.

use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext, WebGlBuffer, WebGlVertexArrayObject};

use super::context::WebGL2Context;

/// Unit quad geometry for sprite rendering
///
/// This creates a simple unit quad from (0,0) to (1,1) with texture coordinates.
/// The vertex shader transforms this quad to the sprite's position/size/rotation.
pub struct QuadGeometry {
    /// Vertex Array Object
    vao: WebGlVertexArrayObject,
    /// Vertex buffer (position + texcoord interleaved)
    #[allow(dead_code)]
    vbo: WebGlBuffer,
    /// Index buffer
    #[allow(dead_code)]
    ibo: WebGlBuffer,
    /// Number of indices to draw
    index_count: i32,
}

impl QuadGeometry {
    /// Create quad geometry for sprite rendering
    pub fn new(context: &WebGL2Context) -> Result<Self, JsValue> {
        let gl = context.gl();

        // Create VAO
        let vao = context.create_vertex_array()?;
        gl.bind_vertex_array(Some(&vao));

        // Vertex data: position (x, y) + texcoord (u, v)
        // Unit quad from (0,0) to (1,1)
        #[rustfmt::skip]
        let vertices: [f32; 16] = [
            // position   texcoord
            0.0, 0.0,     0.0, 0.0,  // bottom-left
            1.0, 0.0,     1.0, 0.0,  // bottom-right
            1.0, 1.0,     1.0, 1.0,  // top-right
            0.0, 1.0,     0.0, 1.0,  // top-left
        ];

        // Indices for two triangles
        let indices: [u16; 6] = [
            0, 1, 2, // first triangle
            0, 2, 3, // second triangle
        ];

        // Create and upload vertex buffer
        let vbo = context.create_buffer()?;
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
        unsafe {
            let vertex_array = js_sys::Float32Array::view(&vertices);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &vertex_array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }

        // Create and upload index buffer
        let ibo = context.create_buffer()?;
        gl.bind_buffer(WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER, Some(&ibo));
        unsafe {
            let index_array = js_sys::Uint16Array::view(&indices);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER,
                &index_array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }

        // Set up vertex attributes
        let stride = 4 * std::mem::size_of::<f32>() as i32; // 4 floats per vertex

        // Position attribute (location = 0)
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(
            0,     // location
            2,     // size (vec2)
            WebGl2RenderingContext::FLOAT,
            false, // normalized
            stride,
            0,     // offset
        );

        // Texcoord attribute (location = 1)
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(
            1,     // location
            2,     // size (vec2)
            WebGl2RenderingContext::FLOAT,
            false, // normalized
            stride,
            (2 * std::mem::size_of::<f32>()) as i32, // offset (after position)
        );

        // Unbind VAO
        gl.bind_vertex_array(None);

        Ok(Self {
            vao,
            vbo,
            ibo,
            index_count: indices.len() as i32,
        })
    }

    /// Bind this geometry for rendering
    pub fn bind(&self, gl: &WebGl2RenderingContext) {
        gl.bind_vertex_array(Some(&self.vao));
    }

    /// Unbind geometry
    pub fn unbind(&self, gl: &WebGl2RenderingContext) {
        gl.bind_vertex_array(None);
    }

    /// Draw the quad (call after binding and setting uniforms)
    pub fn draw(&self, gl: &WebGl2RenderingContext) {
        gl.draw_elements_with_i32(
            WebGl2RenderingContext::TRIANGLES,
            self.index_count,
            WebGl2RenderingContext::UNSIGNED_SHORT,
            0,
        );
    }
}
