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
    vbo_bone_indices: Option<WebGlBuffer>,
    #[allow(dead_code)]
    vbo_bone_weights: Option<WebGlBuffer>,
    #[allow(dead_code)]
    vbo_vertex_colors: Option<WebGlBuffer>,
    #[allow(dead_code)]
    ibo: WebGlBuffer,
    pub index_count: i32,
    pub has_bones: bool,
    pub has_vertex_colors: bool,
    pub has_texcoord2: bool,
    pub texcoord2_direct: bool,
    pub meshdeform_uv_synced: bool,
}

impl Mesh3dBuffers {
    /// Upload mesh data to GPU buffers.
    ///
    /// Vertex attributes:
    /// - Location 0: position (vec3)
    /// - Location 1: normal (vec3)
    /// - Location 2: texcoord (vec2) - primary UV set
    /// - Location 3: texcoord2 (vec2) - secondary UV set (lightmap/shadow)
    /// - Location 4: bone_indices (vec4) - up to 4 bone indices per vertex
    /// - Location 5: bone_weights (vec4) - up to 4 bone weights per vertex
    pub fn new(
        context: &WebGL2Context,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        texcoords: Option<&[[f32; 2]]>,
        texcoords2: Option<&[[f32; 2]]>,
        faces: &[[u32; 3]],
    ) -> Result<Self, JsValue> {
        Self::new_with_bones(context, positions, normals, texcoords, texcoords2, faces, None, None)
    }

    /// Upload mesh data with optional bone indices/weights for skeletal skinning.
    pub fn new_with_bones(
        context: &WebGL2Context,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        texcoords: Option<&[[f32; 2]]>,
        texcoords2: Option<&[[f32; 2]]>,
        faces: &[[u32; 3]],
        bone_indices: Option<&[[f32; 4]]>,
        bone_weights: Option<&[[f32; 4]]>,
    ) -> Result<Self, JsValue> {
        Self::new_full(context, positions, normals, texcoords, texcoords2, faces, bone_indices, bone_weights, None)
    }

    /// Upload mesh data with all optional attributes.
    pub fn new_full(
        context: &WebGL2Context,
        positions: &[[f32; 3]],
        normals: &[[f32; 3]],
        texcoords: Option<&[[f32; 2]]>,
        texcoords2: Option<&[[f32; 2]]>,
        faces: &[[u32; 3]],
        bone_indices: Option<&[[f32; 4]]>,
        bone_weights: Option<&[[f32; 4]]>,
        vertex_colors: Option<&[[f32; 4]]>,
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

        // Bone indices (location 4) - packed as vec4 of floats
        let has_bones = bone_indices.is_some() && bone_weights.is_some();
        let vbo_bone_indices = if let Some(bi) = bone_indices {
            let vbo = context.create_buffer()?;
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
            let bi_flat: Vec<f32> = bi.iter().flat_map(|b| b.iter().copied()).collect();
            unsafe {
                let array = js_sys::Float32Array::view(&bi_flat);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER,
                    &array,
                    WebGl2RenderingContext::STATIC_DRAW,
                );
            }
            gl.enable_vertex_attrib_array(4);
            gl.vertex_attrib_pointer_with_i32(4, 4, WebGl2RenderingContext::FLOAT, false, 0, 0);
            Some(vbo)
        } else {
            gl.disable_vertex_attrib_array(4);
            None
        };

        // Bone weights (location 5) - vec4
        let vbo_bone_weights = if let Some(bw) = bone_weights {
            let vbo = context.create_buffer()?;
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
            let bw_flat: Vec<f32> = bw.iter().flat_map(|b| b.iter().copied()).collect();
            unsafe {
                let array = js_sys::Float32Array::view(&bw_flat);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER,
                    &array,
                    WebGl2RenderingContext::STATIC_DRAW,
                );
            }
            gl.enable_vertex_attrib_array(5);
            gl.vertex_attrib_pointer_with_i32(5, 4, WebGl2RenderingContext::FLOAT, false, 0, 0);
            Some(vbo)
        } else {
            gl.disable_vertex_attrib_array(5);
            None
        };

        // Vertex colors (location 6) - vec4 RGBA
        let vbo_vertex_colors = if let Some(vc) = vertex_colors {
            let vbo = context.create_buffer()?;
            gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
            let vc_flat: Vec<f32> = vc.iter().flat_map(|c| c.iter().copied()).collect();
            unsafe {
                let array = js_sys::Float32Array::view(&vc_flat);
                gl.buffer_data_with_array_buffer_view(
                    WebGl2RenderingContext::ARRAY_BUFFER,
                    &array,
                    WebGl2RenderingContext::STATIC_DRAW,
                );
            }
            gl.enable_vertex_attrib_array(6);
            gl.vertex_attrib_pointer_with_i32(6, 4, WebGl2RenderingContext::FLOAT, false, 0, 0);
            Some(vbo)
        } else {
            gl.disable_vertex_attrib_array(6);
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
            vbo_bone_indices,
            vbo_bone_weights,
            vbo_vertex_colors,
            ibo,
            has_texcoord2: texcoords2.is_some(),
            texcoord2_direct: false,
            meshdeform_uv_synced: false,
            index_count,
            has_bones,
            has_vertex_colors: vertex_colors.is_some(),
        })
    }

    /// Upload or replace the secondary UV set (location 3) for lightmap/shadow coordinates.
    /// Called at render time when meshDeform provides runtime lightmap UVs.
    pub fn update_texcoord2(&mut self, gl: &WebGl2RenderingContext, tc2: &[[f32; 2]]) {
        gl.bind_vertex_array(Some(&self.vao));
        let vbo = if let Some(ref vbo) = self.vbo_texcoords2 {
            vbo.clone()
        } else {
            let vbo = gl.create_buffer().unwrap();
            self.vbo_texcoords2 = Some(vbo.clone());
            vbo
        };
        gl.bind_buffer(WebGl2RenderingContext::ARRAY_BUFFER, Some(&vbo));
        let tc_flat: Vec<f32> = tc2.iter().flat_map(|t| t.iter().copied()).collect();
        unsafe {
            let array = js_sys::Float32Array::view(&tc_flat);
            gl.buffer_data_with_array_buffer_view(
                WebGl2RenderingContext::ARRAY_BUFFER,
                &array,
                WebGl2RenderingContext::DYNAMIC_DRAW,
            );
        }
        gl.enable_vertex_attrib_array(3);
        gl.vertex_attrib_pointer_with_i32(3, 2, WebGl2RenderingContext::FLOAT, false, 0, 0);
        gl.bind_vertex_array(None);
        self.has_texcoord2 = true;
        self.texcoord2_direct = true;
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
