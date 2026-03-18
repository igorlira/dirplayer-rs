//! 3D mesh GPU buffer management for Shockwave 3D rendering
//!
//! Uploads CLOD decoded mesh data (positions, normals, texcoords, faces)
//! to WebGL2 VBOs/IBOs for rendering.

use wasm_bindgen::JsValue;
use web_sys::{WebGl2RenderingContext, WebGlBuffer, WebGlVertexArrayObject};

use super::context::WebGL2Context;

/// GPU buffers for a single 3D mesh
pub struct Mesh3dBuffers {
    vao: WebGlVertexArrayObject,
    #[allow(dead_code)]
    vbo_positions: WebGlBuffer,
    #[allow(dead_code)]
    vbo_normals: WebGlBuffer,
    #[allow(dead_code)]
    vbo_texcoords: Option<WebGlBuffer>,
    #[allow(dead_code)]
    vbo_texcoords2: Option<WebGlBuffer>,
    #[allow(dead_code)]
    ibo: WebGlBuffer,
    pub index_count: i32,
}

impl Mesh3dBuffers {
    /// Upload mesh data to GPU buffers.
    ///
    /// Vertex attributes:
    /// - Location 0: position (vec3)
    /// - Location 1: normal (vec3)
    /// - Location 2: texcoord (vec2) - primary UV set
    /// - Location 3: texcoord2 (vec2) - secondary UV set (lightmap/shadow)
    pub fn new(
        context: &WebGL2Context,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        texcoords: Option<&[[f32; 2]]>,
        texcoords2: Option<&[[f32; 2]]>,
        faces: &[[u32; 3]],
    ) -> Result<Self, JsValue> {
        let gl = context.gl();

        let vao = context.create_vertex_array()?;
        gl.bind_vertex_array(Some(&vao));

        // Positions (location 0)
        let vbo_positions = context.create_buffer()?;
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo_positions));
        let pos_flat: Vec<f32> = positions.iter().flat_map(|p| p.iter().copied()).collect();
        unsafe {
            let array = js_sys::Float32Array::view(&pos_flat);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }
        gl.enable_vertex_attrib_array(0);
        gl.vertex_attrib_pointer_with_i32(0, 3, WebGl2RenderingContext::FLOAT, false, 0, 0);

        // Normals (location 1)
        let vbo_normals = context.create_buffer()?;
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo_normals));
        let norm_flat: Vec<f32> = normals.iter().flat_map(|n| n.iter().copied()).collect();
        unsafe {
            let array = js_sys::Float32Array::view(&norm_flat);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }
        gl.enable_vertex_attrib_array(1);
        gl.vertex_attrib_pointer_with_i32(1, 3, WebGl2RenderingContext::FLOAT, false, 0, 0);

        // TexCoords (location 2) - primary UV set
        let vbo_texcoords = if let Some(tc) = texcoords {
            let vbo = context.create_buffer()?;
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
            let tc_flat: Vec<f32> = tc.iter().flat_map(|t| t.iter().copied()).collect();
            unsafe {
                let array = js_sys::Float32Array::view(&tc_flat);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER,
                    &array,
                    WebGl2RenderingContext::STATIC_DRAW,
                );
            }
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_with_i32(2, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);
            Some(vbo)
        } else {
            gl.disable_vertex_attrib_array(2);
            None
        };

        // TexCoords2 (location 3) - secondary UV set (for lightmap/shadow)
        let vbo_texcoords2 = if let Some(tc2) = texcoords2 {
            let vbo = context.create_buffer()?;
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
            let tc_flat: Vec<f32> = tc2.iter().flat_map(|t| t.iter().copied()).collect();
            unsafe {
                let array = js_sys::Float32Array::view(&tc_flat);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER,
                    &array,
                    WebGl2RenderingContext::STATIC_DRAW,
                );
            }
            gl.enable_vertex_attrib_array(3);
            gl.vertex_attrib_pointer_with_i32(3, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);
            Some(vbo)
        } else {
            gl.disable_vertex_attrib_array(3);
            None
        };

        // Index buffer
        let ibo = context.create_buffer()?;
        gl.bind_buffer(WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER, Some(&ibo));
        let idx_flat: Vec<u32> = faces.iter().flat_map(|f| f.iter().copied()).collect();
        let index_count = idx_flat.len() as i32;
        unsafe {
            let array = js_sys::Uint32Array::view(&idx_flat);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER,
                &array,
                WebGl2RenderingContext::STATIC_DRAW,
            );
        }

        gl.bind_vertex_array(None);

        Ok(Self {
            vao,
            vbo_positions,
            vbo_normals,
            vbo_texcoords,
            vbo_texcoords2,
            ibo,
            index_count,
        })
    }

    /// Bind this mesh for drawing
    pub fn bind(&self, gl: &WebGl2RenderingContext) {
        gl.bind_vertex_array(Some(&self.vao));
        // Explicitly re-bind IBO — some WebGL2 implementations lose the
        // ELEMENT_ARRAY_BUFFER binding from the VAO when other code
        // (e.g. the 2D sprite renderer) changes GL state between frames.
        gl.bind_buffer(WebGl2RenderingContext::ELEMENT_ARRAY_BUFFER, Some(&self.ibo));
    }

    /// Draw the mesh (call after binding and setting uniforms)
    pub fn draw(&self, gl: &WebGl2RenderingContext) {
        gl.draw_elements_with_i32(
            WebGl2RenderingContext::TRIANGLES,
            self.index_count,
            WebGl2RenderingContext::UNSIGNED_INT,
            0,
        );
    }

    /// Unbind
    pub fn unbind(&self, gl: &WebGl2RenderingContext) {
        gl.bind_vertex_array(None);
    }
}
