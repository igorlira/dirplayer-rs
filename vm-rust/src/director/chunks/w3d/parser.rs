/// Top-level W3D file parser. Reads the IFX container format and dispatches to block parsers.
/// Ported from W3DFileParser.cs.

use std::collections::HashMap;
use super::bitstream::IFXBitStreamCompressed;
use super::block_reader::W3dBlockReader;
use super::block_types::*;
use super::clod_decoder::ClodMeshDecoder;
use super::primitives;
use super::types::*;

const W3D_LOG: bool = true;

fn log(msg: &str) {
    if W3D_LOG {
        web_sys::console::log_1(&format!("[W3D] {}", msg).into());
    }
}

struct W3dBlock {
    block_type: u32,
    data: Vec<u8>,
}

pub struct W3dFileParser {
    data: Vec<u8>,
    pos: usize,
    model_resources: HashMap<String, ModelResourceInfo>,
    last_model_resource_name: String,
    clod_decoders: HashMap<String, ClodMeshDecoder>,
    pub scene: W3dScene,
}

impl W3dFileParser {
    pub fn new(data: Vec<u8>) -> Self {
        Self {
            data,
            pos: 0,
            model_resources: HashMap::new(),
            last_model_resource_name: String::new(),
            clod_decoders: HashMap::new(),
            scene: W3dScene::default(),
        }
    }

    pub fn parse(&mut self) -> Result<(), String> {
        if self.data.len() < 16 {
            return Err("W3D file too small".into());
        }

        // Magic: "IFX\0"
        if self.data[0] != 0x49 || self.data[1] != 0x46 || self.data[2] != 0x58 || self.data[3] != 0x00 {
            return Err(format!("Bad W3D magic: {:02X} {:02X} {:02X} {:02X}", self.data[0], self.data[1], self.data[2], self.data[3]));
        }
        self.pos = 4;

        let header_size = self.read_u32_le();
        let _version = self.read_u32_le();
        let _file_size = self.read_u32_le();

        // Skip remaining header bytes
        self.pos = 4 + 4 + header_size as usize;

        // Parse blocks
        let mut block_index = 0;
        while self.pos + 8 <= self.data.len() {
            let block_type = self.read_u32_le();
            let mut block_size = self.read_u32_le();

            if self.pos + block_size as usize > self.data.len() {
                block_size = (self.data.len() - self.pos) as u32;
            }

            let block_data = self.data[self.pos..self.pos + block_size as usize].to_vec();
            self.pos += block_size as usize;

            let block = W3dBlock {
                block_type,
                data: block_data,
            };

            let name = block_type_name(block_type);
            if name != "Unknown" {
                log(&format!("Block {} type=0x{:08X} ({}) size={}", block_index, block_type, name, block_size));
            }

            if let Err(e) = self.parse_block(&block) {
                log(&format!("Block {} parse error: {}", block_index, e));
            }

            // Align to 4 bytes
            self.pos = (self.pos + 3) & !3;
            block_index += 1;
        }

        // Finalize: extract decoded CLOD meshes at full resolution (all patches applied)
        for (name, decoder) in &self.clod_decoders {
            let meshes = decoder.get_decoded_meshes_full_resolution();
            for (mi, m) in meshes.iter().enumerate() {
                web_sys::console::log_1(&format!(
                    "[W3D] CLOD mesh '{}': {} verts, {} faces, {} bone_idx, {} bone_wgt (full res with patches)",
                    name, m.positions.len(), m.faces.len(), m.bone_indices.len(), m.bone_weights.len()
                ).into());
                // Log bounding box to diagnose centering issues
                if !m.positions.is_empty() {
                    let (mut min_x, mut min_y, mut min_z) = (f32::MAX, f32::MAX, f32::MAX);
                    let (mut max_x, mut max_y, mut max_z) = (f32::MIN, f32::MIN, f32::MIN);
                    for p in &m.positions {
                        min_x = min_x.min(p[0]); min_y = min_y.min(p[1]); min_z = min_z.min(p[2]);
                        max_x = max_x.max(p[0]); max_y = max_y.max(p[1]); max_z = max_z.max(p[2]);
                    }
                    let cx = (min_x + max_x) / 2.0;
                    let cy = (min_y + max_y) / 2.0;
                    let cz = (min_z + max_z) / 2.0;
                    web_sys::console::log_1(&format!(
                        "[W3D] CLOD mesh '{}' sub{}: bbox=({:.1},{:.1},{:.1})→({:.1},{:.1},{:.1}) center=({:.1},{:.1},{:.1}) size=({:.1},{:.1},{:.1})",
                        name, mi, min_x, min_y, min_z, max_x, max_y, max_z, cx, cy, cz,
                        max_x-min_x, max_y-min_y, max_z-min_z
                    ).into());
                }
            }
            self.scene.clod_meshes.insert(name.clone(), meshes);
        }

        // Store CLOD decoders for runtime LOD changes
        self.scene.clod_decoders = std::mem::take(&mut self.clod_decoders);

        // Copy model resources to scene
        self.scene.model_resources = self.model_resources.clone();

        // Ensure DefaultShader exists (Director always has one)
        if !self.scene.shaders.iter().any(|s| s.name == "DefaultShader") {
            self.scene.shaders.insert(0, W3dShader {
                name: "DefaultShader".to_string(),
                ..Default::default()
            });
        }

        // Director built-in "defaultmodel" plane resource (used by overlay/HUD scripts)
        if !self.scene.clod_meshes.contains_key("defaultmodel") {
            use super::types::ClodDecodedMesh;
            // Single back-face mesh with U-mirrored UVs.
            // Director's insceneoverlay/3d_textsprite rotate the model 180° around Y,
            // which flips X and makes the back face visible. The U-mirrored UVs compensate
            // for the X-flip, producing correctly oriented text.
            // Using only one mesh avoids Z-fighting between front/back faces.
            // UVs in IFX [-0.5, 0.5] range for CLOD remap: u_out = u_in + 0.5, v_out = 0.5 - v_in
            let plane = ClodDecodedMesh {
                name: "defaultmodel".to_string(),
                positions: vec![[-0.5,-0.5,0.0],[0.5,-0.5,0.0],[0.5,0.5,0.0],[-0.5,0.5,0.0]],
                normals: vec![[0.0,0.0,-1.0]; 4],
                tex_coords: vec![vec![[0.5,-0.5],[-0.5,-0.5],[-0.5,0.5],[0.5,0.5]]],
                faces: vec![[0,2,1],[0,3,2]],
                diffuse_colors: vec![], specular_colors: vec![],
                bone_indices: vec![], bone_weights: vec![],
            };
            self.scene.clod_meshes.insert("defaultmodel".to_string(), vec![plane]);
            // Also register in model_resources so modelResource("defaultmodel") lookups work
            if !self.scene.model_resources.contains_key("defaultmodel") {
                self.scene.model_resources.insert("defaultmodel".to_string(), ModelResourceInfo {
                    name: "defaultmodel".to_string(),
                    mesh_infos: vec![],
                    max_resolution: 0,
                    shading_count: 1,
                    shader_bindings: vec![],
                    pos_iq: 1.0, norm_iq: 1.0, normal_crease: 1.0, tc_iq: 1.0, diff_iq: 1.0, spec_iq: 1.0,
                    has_distal_edge_merge: false, has_neighbor_mesh: false,
                    uv_gen_mode: None, sync_table: None, distal_edge_merges: None,
                });
            }
        }

        // Director always creates a "UIAmbient" light (black ambient, no visual contribution)
        // so Lingo scripts can reference it by name.
        if !self.scene.lights.iter().any(|l| l.name == "UIAmbient") {
            self.scene.lights.push(W3dLight {
                name: "UIAmbient".to_string(),
                light_type: W3dLightType::Ambient,
                color: [0.0, 0.0, 0.0],
                enabled: true,
                spot_angle: 90.0,
                attenuation: [1.0, 0.0, 0.0],
            });
            self.scene.nodes.push(W3dNode {
                name: "UIAmbient".to_string(),
                node_type: W3dNodeType::Light,
                parent_name: "World".to_string(),
                ..Default::default()
            });
        }

        log(&format!("Parse complete: {} materials, {} shaders, {} nodes, {} lights, {} textures, {} skeletons, {} motions, {} mesh resources",
            self.scene.materials.len(), self.scene.shaders.len(), self.scene.nodes.len(),
            self.scene.lights.len(), self.scene.texture_images.len(),
            self.scene.skeletons.len(), self.scene.motions.len(), self.scene.clod_meshes.len()));

        Ok(())
    }

    fn parse_block(&mut self, block: &W3dBlock) -> Result<(), String> {
        let mut r = W3dBlockReader::new(&block.data);

        match block.block_type {
            FILE_HEADER => { /* 3 flag bytes, skip */ }

            MATERIAL | MATERIAL_V1 => self.parse_material(&mut r)?,

            LIGHT_RESOURCE => self.parse_light_resource(&mut r)?,

            GROUP_NODE => self.parse_group_node(&mut r, true)?,
            GROUP_NODE_V1 => self.parse_group_node(&mut r, false)?,

            LIGHT_NODE => self.parse_light_node(&mut r, true)?,
            LIGHT_NODE_V1 => self.parse_light_node(&mut r, false)?,

            MODEL_NODE => self.parse_model_node(&mut r, true)?,
            MODEL_NODE_V1 => self.parse_model_node(&mut r, false)?,

            VIEW_NODE => self.parse_view_node(&mut r, true)?,
            VIEW_NODE_V1 => self.parse_view_node(&mut r, false)?,

            SHADER_LIT_TEXTURE_V0 | SHADER_LIT_TEXTURE_V1 | SHADER_LIT_TEXTURE =>
                self.parse_shader_lit_texture(&mut r, block.block_type)?,

            TEXTURE_DECL => self.parse_texture_declaration(&mut r)?,
            TEXTURE_CONT => self.parse_texture_continuation(&mut r)?,
            TEXTURE_INFO => self.parse_texture_info(&mut r)?,

            MODEL_BLOCK2 => self.parse_model_block2(&mut r)?,

            RAW_MESH => self.parse_raw_mesh(&mut r)?,
            BONES_BLOCK => self.parse_bones_block(&mut r)?,
            MOTION_BLOCK => self.parse_motion_block(&mut r)?,

            COMP_SYNCH_TABLE => self.parse_comp_synch_table(&block.data)?,
            DISTAL_EDGE_MERGE => self.parse_distal_edge_merge(&block.data)?,
            COMPRESSED_GEOM => self.parse_compressed_geom(&block.data)?,

            PLANE => { let _ = primitives::parse_plane(&mut r); }
            BOX => { let _ = primitives::parse_box(&mut r); }
            SPHERE => { let _ = primitives::parse_sphere(&mut r); }
            CYLINDER | CYLINDER2 => { let _ = primitives::parse_cylinder(&mut r); }
            PARTICLE_SYS => { let _ = primitives::parse_particle_system(&mut r); }
            GLYPH_3D => { let _ = primitives::parse_glyph_3d(&mut r); }
            PHYSICS_MESH => { let _ = primitives::parse_physics_mesh(&mut r); }

            // Skeleton modifiers, shader variants, and other modifier blocks
            // Parse minimally to consume bitstream correctly
            SKELETON_MODIFIER | MODIFIER_PARAM_92 | MODIFIER_PARAM_97 =>
                self.parse_skeleton_modifier(&mut r)?,
            MODIFIER_PARAM_94 => self.parse_modifier_param(&mut r, "MRM")?,
            MODIFIER_PARAM_95 => self.parse_modifier_param(&mut r, "Physics")?,
            SHADER_PAINTER_V0 | SHADER_PAINTER | SHADER_INKER |
            SHADER_ENGRAVER | SHADER_NEWSPRINT | SHADER_PARTICLE =>
                self.parse_npr_shader(&mut r, block.block_type)?,
            UV_GENERATOR => self.parse_uv_generator(&mut r)?,
            SUBDIV_SURFACE => self.parse_subdiv_surface(&mut r)?,
            PHYSICS_MODIFIER => self.parse_modifier_param(&mut r, "PhysicsMod")?,
            DEFORM_MODIFIER => self.parse_modifier_param(&mut r, "DeformMod")?,
            INKER_MODIFIER => self.parse_inker_modifier(&mut r)?,
            ORIG_TEX_COORDS => self.parse_extra_texcoords(&mut r, "OrigTexCoords")?,
            TEX_COORDS => self.parse_extra_texcoords(&mut r, "TexCoords")?,

            CONTEXT_SEP => { /* Context separator */ }
            _ => {}
        }

        Ok(())
    }

    // ─── Block Parsers ───

    fn parse_material(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let attrs = r.read_u32()?;

        let mut mat = W3dMaterial { name: name.clone(), ..Default::default() };
        if (attrs & 0x01) != 0 { mat.ambient = r.read_color_rgba()?; }
        if (attrs & 0x02) != 0 { mat.diffuse = r.read_color_rgba()?; }
        if (attrs & 0x04) != 0 { mat.specular = r.read_color_rgba()?; }
        if (attrs & 0x08) != 0 { mat.emissive = r.read_color_rgba()?; }
        if (attrs & 0x10) != 0 { mat.reflectivity = r.read_f32()?; }
        if (attrs & 0x20) != 0 { mat.opacity = r.read_f32()?; }
        if (attrs & 0x40) != 0 { let _ = r.read_f32()?; } // reserved
        if (attrs & 0x80) != 0 { mat.shininess = r.read_f32()?; }

        log(&format!("  Material: \"{}\" diffuse=({:.2},{:.2},{:.2}) opacity={:.2} reflectivity={:.4} shininess={:.4} attrs=0x{:02X}", name, mat.diffuse[0], mat.diffuse[1], mat.diffuse[2], mat.opacity, mat.reflectivity, mat.shininess, attrs));
        self.scene.materials.push(mat);
        Ok(())
    }

    fn parse_light_resource(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let light_type_raw = r.read_u8()?;
        let enabled = r.read_u8()? != 0;
        let cr = r.read_f32()?;
        let cg = r.read_f32()?;
        let cb = r.read_f32()?;
        let a0 = r.read_f32()?;
        let a1 = r.read_f32()?;
        let a2 = r.read_f32()?;
        let spot_angle = r.read_f32()?;
        let _reserved = r.read_f32()?;

        let light_type = match light_type_raw {
            0 => W3dLightType::Ambient,
            1 => W3dLightType::Directional,
            2 => W3dLightType::Point,
            3 => W3dLightType::Spot,
            _ => W3dLightType::Point,
        };

        log(&format!("  Light: \"{}\" type={:?} color=({:.2},{:.2},{:.2})", name, light_type, cr, cg, cb));
        self.scene.lights.push(W3dLight {
            name,
            light_type,
            color: [cr, cg, cb],
            attenuation: [a0, a1, a2],
            spot_angle,
            enabled,
        });
        Ok(())
    }

    fn parse_node_header(&self, r: &mut W3dBlockReader, has_bounds: bool) -> Result<[f32; 16], String> {
        let matrix = r.read_matrix4x4()?;
        if has_bounds {
            let _bound_sphere = r.read_vec4()?;
            let _bound_box = r.read_vec4()?;
        }
        Ok(matrix)
    }

    fn parse_group_node(&mut self, r: &mut W3dBlockReader, has_bounds: bool) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let parent = r.read_ifx_string()?;
        let resource = r.read_ifx_string()?;
        let transform = self.parse_node_header(r, has_bounds)?;

        log(&format!("  GroupNode: \"{}\" parent=\"{}\"", name, parent));
        self.scene.nodes.push(W3dNode {
            name,
            parent_name: parent,
            resource_name: resource,
            node_type: W3dNodeType::Group,
            transform,
            ..Default::default()
        });
        Ok(())
    }

    fn parse_light_node(&mut self, r: &mut W3dBlockReader, has_bounds: bool) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let parent = r.read_ifx_string()?;
        let resource = r.read_ifx_string()?;
        let transform = self.parse_node_header(r, has_bounds)?;
        if r.remaining() >= 2 { let _light_res = r.read_ifx_string()?; }

        self.scene.nodes.push(W3dNode {
            name,
            parent_name: parent,
            resource_name: resource,
            node_type: W3dNodeType::Light,
            transform,
            ..Default::default()
        });
        Ok(())
    }

    fn parse_model_node(&mut self, r: &mut W3dBlockReader, has_bounds: bool) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let parent = r.read_ifx_string()?;
        let resource = r.read_ifx_string()?;
        let transform = self.parse_node_header(r, has_bounds)?;

        let mut node = W3dNode {
            name: name.clone(),
            parent_name: parent,
            resource_name: resource,
            node_type: W3dNodeType::Model,
            transform,
            ..Default::default()
        };

        if r.remaining() >= 2 { node.model_resource_name = r.read_ifx_string()?; }
        if r.remaining() >= 2 { let _style = r.read_ifx_string()?; }
        if r.remaining() >= 4 { let _render_pass = r.read_u32()?; }
        if r.remaining() >= 2 { node.shader_name = r.read_ifx_string()?; }

        log(&format!("  ModelNode: \"{}\" resource=\"{}\"", name, node.model_resource_name));
        self.scene.nodes.push(node);
        Ok(())
    }

    fn parse_view_node(&mut self, r: &mut W3dBlockReader, has_bounds: bool) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let parent = r.read_ifx_string()?;
        let resource = r.read_ifx_string()?;
        let transform = self.parse_node_header(r, has_bounds)?;

        let mut node = W3dNode {
            name: name.clone(),
            parent_name: parent,
            resource_name: resource,
            node_type: W3dNodeType::View,
            transform,
            ..Default::default()
        };

        let view_attrs = if r.remaining() >= 4 { r.read_u32()? } else { 0 };
        if r.remaining() >= 12 {
            node.near_plane = r.read_f32()?;
            node.far_plane = r.read_f32()?;
            node.fov = r.read_f32()?;
        }

        log(&format!(
            "  ViewNode: \"{}\" parent=\"{}\" viewAttrs=0x{:X} near={} far={} fov={}\n    pos: ({:.3},{:.3},{:.3})",
            node.name, node.parent_name, view_attrs, node.near_plane, node.far_plane, node.fov,
            transform[12], transform[13], transform[14],
        ));

        // Skip remaining view data
        self.scene.nodes.push(node);
        Ok(())
    }

    fn parse_shader_lit_texture(&mut self, r: &mut W3dBlockReader, block_type: u32) -> Result<(), String> {
        let is_v200 = block_type == SHADER_LIT_TEXTURE; // -200
        let name = r.read_ifx_string()?;
        let attrs = r.read_u32()?;
        let render_pass = r.read_u32()?;

        let mut shader = W3dShader {
            name: name.clone(),
            attrs,
            render_pass,
            ..Default::default()
        };

        if (attrs & 0x01) != 0 {
            shader.material_name = r.read_ifx_string()?;
        }

        for layer in 0..8u32 {
            if (attrs & (1 << (16 + layer))) != 0 {
                let tex_name = r.read_ifx_string()?;
                let intensity = r.read_f32()?;
                let blend_func = r.read_u8()?;
                let blend_src = r.read_u8()?;
                let blend_const = r.read_f32()?;
                let tex_mode = r.read_u8()?;
                let tex_transform = r.read_matrix4x4()?;
                let wrap_transform = r.read_matrix4x4()?;
                let repeat_s = r.read_u8()?;
                let repeat_t = if is_v200 { r.read_u8()? } else { 1 };

                shader.texture_layers.push(W3dTextureLayer {
                    name: tex_name,
                    intensity,
                    blend_func,
                    blend_src,
                    blend_const,
                    tex_mode,
                    tex_transform,
                    wrap_transform,
                    repeat_s,
                    repeat_t,
                });
            }
        }

        log(&format!("  Shader: \"{}\" material=\"{}\" layers={}", name, shader.material_name, shader.texture_layers.len()));
        self.scene.shaders.push(shader);
        Ok(())
    }

    /// Parse NPR shader blocks (Painter, Inker, Engraver, Newsprint, Particle).
    /// These share the LitTexture base format (name, attrs, material, texture layers).
    /// We parse what we can and tag the shader type for the renderer.
    fn parse_npr_shader(&mut self, r: &mut W3dBlockReader, block_type: u32) -> Result<(), String> {
        use crate::director::chunks::w3d::types::W3dShaderType;

        let shader_type = match block_type {
            SHADER_PAINTER_V0 | SHADER_PAINTER => W3dShaderType::Painter,
            SHADER_INKER => W3dShaderType::Inker,
            SHADER_ENGRAVER => W3dShaderType::Engraver,
            SHADER_NEWSPRINT => W3dShaderType::Newsprint,
            SHADER_PARTICLE => W3dShaderType::Particle,
            _ => W3dShaderType::LitTexture,
        };

        // NPR shaders start with the same header as LitTexture
        if r.remaining() < 4 { return Ok(()); } // too short
        let name = match r.read_ifx_string() {
            Ok(s) => s,
            Err(_) => return Ok(()), // malformed — skip gracefully
        };
        let attrs = if r.remaining() >= 4 { r.read_u32()? } else { 0 };
        let render_pass = if r.remaining() >= 4 { r.read_u32()? } else { 0 };

        let mut shader = W3dShader {
            name: name.clone(),
            attrs,
            render_pass,
            shader_type,
            ..Default::default()
        };

        // Try to read material name
        if (attrs & 0x01) != 0 && r.remaining() >= 2 {
            shader.material_name = r.read_ifx_string().unwrap_or_default();
        }

        // Try to read texture layers (same format as LitTexture)
        let is_v200 = block_type == SHADER_PAINTER;
        for layer in 0..8u32 {
            if (attrs & (1 << (16 + layer))) == 0 { continue; }
            if r.remaining() < 10 { break; } // not enough data
            let tex_name = match r.read_ifx_string() {
                Ok(s) => s,
                Err(_) => break,
            };
            if r.remaining() < 4 + 1 + 1 + 4 + 1 + 64 + 64 + 1 { break; }
            let intensity = r.read_f32()?;
            let blend_func = r.read_u8()?;
            let blend_src = r.read_u8()?;
            let blend_const = r.read_f32()?;
            let tex_mode = r.read_u8()?;
            let tex_transform = r.read_matrix4x4()?;
            let wrap_transform = r.read_matrix4x4()?;
            let repeat_s = r.read_u8()?;
            let repeat_t = if is_v200 { r.read_u8().unwrap_or(1) } else { 1 };

            shader.texture_layers.push(W3dTextureLayer {
                name: tex_name,
                intensity,
                blend_func,
                blend_src,
                blend_const,
                tex_mode,
                tex_transform,
                wrap_transform,
                repeat_s,
                repeat_t,
            });
        }

        log(&format!("  NPR Shader ({:?}): \"{}\" material=\"{}\" layers={}",
            shader.shader_type, name, shader.material_name, shader.texture_layers.len()));
        self.scene.shaders.push(shader);
        Ok(())
    }

    fn parse_texture_declaration(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let _name = r.read_ifx_string()?;
        Ok(())
    }

    fn parse_texture_continuation(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let _cont_index = r.read_u8()?;
        let image_size = r.remaining();
        if image_size > 0 {
            let image_data = r.read_bytes(image_size)?;
            let entry = self.scene.texture_images.entry(name.clone()).or_insert_with(Vec::new);
            entry.extend_from_slice(&image_data);

            let format = if image_data.len() >= 2 && image_data[0] == 0xFF && image_data[1] == 0xD8 {
                "JPEG"
            } else if image_data.len() >= 2 && image_data[0] == 0x89 && image_data[1] == 0x50 {
                "PNG"
            } else {
                "unknown"
            };
            let total = entry.len();
            log(&format!("  Texture: \"{}\" cont={} chunk={} bytes total={} bytes ({})", name, _cont_index, image_size, total, format));
        }
        Ok(())
    }

    fn parse_texture_info(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let render_format = r.read_u8()?;
        let mip_mode = r.read_u8()?;
        let mag_filter = r.read_u8()?;
        let image_type = r.read_u8()?;
        self.scene.texture_infos.push(W3dTextureInfo { name, render_format, mip_mode, mag_filter, image_type });
        Ok(())
    }

    fn parse_model_block2(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let description_attrs = r.read_u32()?;
        let has_neighbor_mesh = (description_attrs & 2) != 0;
        let num_meshes = r.read_u32()?;

        let mut res_info = ModelResourceInfo {
            name: name.clone(),
            has_neighbor_mesh,
            ..Default::default()
        };

        for _ in 0..num_meshes {
            let num_vertices = r.read_u32()?;
            let num_faces = r.read_u32()?;
            let update_data_count = r.read_u32()?;
            let num_updates = r.read_u32()?;
            let inverse_sync_bias = r.read_f32()?;
            let vertex_attributes = r.read_u32()?;

            res_info.mesh_infos.push(ClodMeshInfo {
                num_vertices,
                num_faces,
                num_updates,
                update_data_count,
                inverse_sync_bias,
                vertex_attributes,
            });
        }

        // Shader list
        let num_shaders = r.read_u32()?;
        res_info.shading_count = num_shaders;
        for _ in 0..num_shaders {
            if r.remaining() < 2 { break; }
            let shader_name = r.read_ifx_string()?;
            let mut binding = ModelShaderBinding { name: shader_name, mesh_bindings: Vec::new() };
            for _ in 0..num_meshes {
                if r.remaining() < 2 { break; }
                binding.mesh_bindings.push(r.read_ifx_string()?);
            }
            res_info.shader_bindings.push(binding);
        }

        // Bounding sphere
        if r.remaining() >= 16 {
            let _bx = r.read_f32()?;
            let _by = r.read_f32()?;
            let _bz = r.read_f32()?;
            let _br = r.read_f32()?;
        }

        // Quality factors
        if r.remaining() >= 24 {
            res_info.normal_crease = r.read_f32()?;
            res_info.pos_iq = r.read_f32()?;
            res_info.norm_iq = r.read_f32()?;
            res_info.tc_iq = r.read_f32()?;
            res_info.diff_iq = r.read_f32()?;
            res_info.spec_iq = r.read_f32()?;
        }

        if r.remaining() >= 4 {
            res_info.max_resolution = r.read_u32()?;
        }

        log(&format!("  ModelResource: \"{}\" meshes={} maxRes={} descAttrs=0x{:X} nbr={}",
            name, num_meshes, res_info.max_resolution, description_attrs, has_neighbor_mesh));

        self.last_model_resource_name = name.clone();
        self.model_resources.insert(name, res_info);
        Ok(())
    }

    fn parse_raw_mesh(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = r.read_ifx_string()?;
        let chain_index = r.read_u32()?;

        let num_faces = r.read_u32()?;
        let num_positions = r.read_u32()?;
        let num_normals = r.read_u32()?;
        let num_vertex_colors = r.read_u32()?;
        let num_tex_coords = r.read_u32()?;
        let _num_tex_layers = r.read_u8()?;

        log(&format!("  RawMesh: \"{}\" faces={} pos={} norm={} tc={}",
            name, num_faces, num_positions, num_normals, num_tex_coords));

        // Read face indices (3 u32 per face for position indices)
        let mut faces = Vec::with_capacity(num_faces as usize);
        for _ in 0..num_faces {
            if r.remaining() < 12 { break; }
            let a = r.read_u32()?;
            let b = r.read_u32()?;
            let c = r.read_u32()?;
            faces.push([a, b, c]);
        }

        // Read positions
        let mut positions = Vec::with_capacity(num_positions as usize);
        for _ in 0..num_positions {
            if r.remaining() < 12 { break; }
            let x = r.read_f32()?;
            let y = r.read_f32()?;
            let z = r.read_f32()?;
            positions.push([x, y, z]);
        }

        // Read normals
        let mut normals = Vec::with_capacity(num_normals as usize);
        for _ in 0..num_normals {
            if r.remaining() < 12 { break; }
            let x = r.read_f32()?;
            let y = r.read_f32()?;
            let z = r.read_f32()?;
            normals.push([x, y, z]);
        }

        // Skip vertex colors
        if num_vertex_colors > 0 {
            r.skip((num_vertex_colors * 16) as usize); // 4 floats per color
        }

        // Read texcoords
        let mut tex_coords = Vec::with_capacity(num_tex_coords as usize);
        for _ in 0..num_tex_coords {
            if r.remaining() < 8 { break; }
            let u = r.read_f32()?;
            let v = r.read_f32()?;
            tex_coords.push([u, v]);
        }

        self.scene.raw_meshes.push(W3dRawMesh {
            name,
            chain_index,
            positions,
            normals,
            tex_coords,
            vertex_colors: Vec::new(),
            faces,
        });
        Ok(())
    }

    fn parse_bones_block(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let skel_name = r.read_ifx_string()?;
        let num_bones = r.read_u32()?;

        let mut skeleton = W3dSkeleton { name: skel_name.clone(), bones: Vec::with_capacity(num_bones as usize) };

        for _ in 0..num_bones {
            if r.remaining() < 2 { break; }
            let bone_name = r.read_ifx_string()?;
            let parent_idx = r.read_u32()?;
            let length = r.read_f32()?;
            let dx = r.read_f32()?;
            let dy = r.read_f32()?;
            let dz = r.read_f32()?;
            // Skeleton bone quaternion: [W,X,Y,Z] (verified from IFX + GLTF rest pose test)
            let qw = r.read_f32()?;
            let qx = r.read_f32()?;
            let qy = r.read_f32()?;
            let qz = r.read_f32()?;
            let bone_attrs = r.read_u32()?;

            skeleton.bones.push(W3dBone {
                name: bone_name,
                parent_index: if parent_idx == 0xFFFFFFFF { -1 } else { parent_idx as i32 },
                length,
                dir_x: dx,
                dir_y: dy,
                dir_z: dz,
                rot_x: qx,
                rot_y: qy,
                rot_z: qz,
                rot_w: qw,
                attributes: bone_attrs,
            });
        }

        log(&format!("  Skeleton: \"{}\" bones={}", skel_name, skeleton.bones.len()));
        self.scene.skeletons.push(skeleton);
        Ok(())
    }

    fn parse_motion_block(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let block_data = r.read_bytes(r.remaining())?;
        let mut bs = IFXBitStreamCompressed::new(&block_data);

        let motion_name = bs.read_ifx_string();
        let track_count = bs.read_u32();
        let time_iq = bs.read_f32();
        let rot_iq = bs.read_f32();

        let mut motion = W3dMotion { name: motion_name.clone(), tracks: Vec::with_capacity(track_count as usize) };

        for _ in 0..track_count {
            let bone_name = bs.read_ifx_string();
            let keyframe_count = bs.read_u32();

            if keyframe_count > 0x5D1745D {
                break; // safety check from reference
            }

            let pos_iq = bs.read_f32();
            let scale_iq = bs.read_f32();

            let mut track = W3dMotionTrack {
                bone_name,
                keyframes: Vec::with_capacity(keyframe_count as usize),
            };

            // Accumulators for delta-coding
            let (mut acc_time, mut acc_pos_x, mut acc_pos_y, mut acc_pos_z) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
            let (mut acc_scale_x, mut acc_scale_y, mut acc_scale_z) = (1.0f32, 1.0f32, 1.0f32);
            let (mut acc_rot_w, mut acc_rot_x, mut acc_rot_y, mut acc_rot_z) = (1.0f32, 0.0f32, 0.0f32, 0.0f32);

            for k in 0..keyframe_count {
                let is_first_or_last = k == 0 || k == keyframe_count - 1;

                // Time
                let delta_time = if is_first_or_last {
                    bs.read_f32()
                } else {
                    let time_sign = bs.read_compressed_u8(1);
                    let time_mag = bs.read_compressed_u32(2);
                    let mut dt = time_mag as f32 * time_iq;
                    if (time_sign & 1) != 0 { dt = -dt; }
                    dt
                };
                acc_time += delta_time;

                // Position
                if k == 0 {
                    acc_pos_x = bs.read_f32();
                    acc_pos_y = bs.read_f32();
                    acc_pos_z = bs.read_f32();
                } else {
                    let pos_sign = bs.read_compressed_u8(3);
                    let mag_x = bs.read_compressed_u32(4) as f32 * pos_iq;
                    let mag_y = bs.read_compressed_u32(4) as f32 * pos_iq;
                    let mag_z = bs.read_compressed_u32(4) as f32 * pos_iq;
                    acc_pos_x += if (pos_sign & 1) != 0 { -mag_x } else { mag_x };
                    acc_pos_y += if (pos_sign & 2) != 0 { -mag_y } else { mag_y };
                    acc_pos_z += if (pos_sign & 4) != 0 { -mag_z } else { mag_z };
                }

                // Rotation (quaternion) — first keyframe: [W,X,Y,Z]
                // (verified from IFX decompiled code + C# GLTF testing)
                let (qw, qx, qy, qz);
                if k == 0 {
                    qw = bs.read_f32();
                    qx = bs.read_f32();
                    qy = bs.read_f32();
                    qz = bs.read_f32();
                } else {
                    let rot_sign = bs.read_compressed_u8(5);
                    let rot_mag1 = bs.read_compressed_u32(6);
                    let rot_mag2 = bs.read_compressed_u32(6);
                    let rot_mag3 = bs.read_compressed_u32(6);
                    let mut dx = (rot_mag1 as f32 * rot_iq).min(1.0);
                    let mut dy = (rot_mag2 as f32 * rot_iq).min(1.0);
                    let mut dz = (rot_mag3 as f32 * rot_iq).min(1.0);
                    let mut dw = (1.0 - dx * dx - dy * dy - dz * dz).abs().sqrt();

                    if (rot_sign & 1) != 0 { dw = -dw; }
                    if (rot_sign & 2) != 0 { dx = -dx; }
                    if (rot_sign & 4) != 0 { dy = -dy; }
                    if (rot_sign & 8) != 0 { dz = -dz; }

                    // Hamilton product: q_new = q_prev * q_delta (standard order)
                    let (pw, px, py, pz) = (acc_rot_w, acc_rot_x, acc_rot_y, acc_rot_z);
                    qw = pw * dw - px * dx - py * dy - pz * dz;
                    qx = pw * dx + px * dw + py * dz - pz * dy;
                    qy = pw * dy - px * dz + py * dw + pz * dx;
                    qz = pw * dz + px * dy - py * dx + pz * dw;
                }
                acc_rot_w = qw;
                acc_rot_x = qx;
                acc_rot_y = qy;
                acc_rot_z = qz;

                // Scale
                if k == 0 {
                    acc_scale_x = bs.read_f32();
                    acc_scale_y = bs.read_f32();
                    acc_scale_z = bs.read_f32();
                } else {
                    let sc_sign = bs.read_compressed_u8(7);
                    let mag_x = bs.read_compressed_u32(8) as f32 * scale_iq;
                    let mag_y = bs.read_compressed_u32(8) as f32 * scale_iq;
                    let mag_z = bs.read_compressed_u32(8) as f32 * scale_iq;
                    acc_scale_x += if (sc_sign & 1) != 0 { -mag_x } else { mag_x };
                    acc_scale_y += if (sc_sign & 2) != 0 { -mag_y } else { mag_y };
                    acc_scale_z += if (sc_sign & 4) != 0 { -mag_z } else { mag_z };
                }

                track.keyframes.push(W3dKeyframe {
                    time: acc_time,
                    pos_x: acc_pos_x,
                    pos_y: acc_pos_y,
                    pos_z: acc_pos_z,
                    rot_w: qw,
                    rot_x: qx,
                    rot_y: qy,
                    rot_z: qz,
                    scale_x: acc_scale_x,
                    scale_y: acc_scale_y,
                    scale_z: acc_scale_z,
                });
            }

            motion.tracks.push(track);
        }

        log(&format!("  Motion: \"{}\" tracks={} duration={:.3}s", motion_name, motion.tracks.len(), motion.duration()));
        self.scene.motions.push(motion);
        Ok(())
    }

    fn parse_comp_synch_table(&mut self, data: &[u8]) -> Result<(), String> {
        let mut bs = IFXBitStreamCompressed::new(data);
        let name = bs.read_ifx_string();

        let res_info = match self.model_resources.get_mut(&name) {
            Some(r) => r,
            None => return Ok(()),
        };

        let num_meshes = res_info.mesh_infos.len();
        let mut sync_table = Vec::with_capacity(num_meshes);

        for m in 0..num_meshes {
            let num_updates = res_info.mesh_infos[m].num_updates;
            let mut table = Vec::with_capacity(num_updates as usize);
            let mut accum = 0u32;

            for _ in 0..num_updates {
                let delta = bs.read_compressed_u32(1);
                accum += delta;
                table.push(accum);
            }
            sync_table.push(table);
        }

        res_info.sync_table = Some(sync_table);
        log(&format!("  SyncTable for \"{}\" ({} meshes)", name, num_meshes));
        Ok(())
    }

    fn parse_distal_edge_merge(&mut self, data: &[u8]) -> Result<(), String> {
        let mut bs = IFXBitStreamCompressed::new(data);
        let name = bs.read_ifx_string();

        let res_info = match self.model_resources.get_mut(&name) {
            Some(r) => r,
            None => return Ok(()),
        };

        res_info.has_distal_edge_merge = true;
        let resolution_count = bs.read_u32();
        let mut merges_list = Vec::with_capacity(resolution_count as usize);

        for _ in 0..resolution_count {
            let merge_count = bs.read_compressed_u32(1);
            let mut merges = Vec::with_capacity(merge_count as usize);

            for _ in 0..merge_count {
                let mesh_a = bs.read_compressed_u32(2);
                let face_a = bs.read_u32();
                let corner_a = bs.read_compressed_u32(3) % 3;
                let mesh_b = bs.read_compressed_u32(4);
                let face_b = bs.read_u32();
                let corner_b = bs.read_compressed_u32(5) % 3;

                merges.push(DistalEdgeMergeRecord { mesh_a, face_a, corner_a, mesh_b, face_b, corner_b });
            }

            merges_list.push(merges);
        }

        res_info.distal_edge_merges = Some(merges_list);
        log(&format!("  DistalEdgeMerge for \"{}\" ({} resolutions)", name, resolution_count));
        Ok(())
    }

    fn parse_compressed_geom(&mut self, data: &[u8]) -> Result<(), String> {
        // Peek name from block data to look up correct model resource
        let mut peek_bs = IFXBitStreamCompressed::new(data);
        let clod_name = peek_bs.read_ifx_string();

        // Get or create decoder for this resource
        if !self.clod_decoders.contains_key(&clod_name) {
            let mut decoder = ClodMeshDecoder::new();
            if let Some(res_info) = self.model_resources.get(&clod_name) {
                decoder.set_mesh_infos(res_info);
            }
            self.clod_decoders.insert(clod_name.clone(), decoder);
        }

        let decoder = self.clod_decoders.get_mut(&clod_name).unwrap();
        decoder.decode_block(data)?;

        Ok(())
    }

    fn parse_uv_generator(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        // UV Generator block: contains orientation mode + transform matrix
        if r.remaining() < 4 { return Ok(()); }
        let name = r.read_ifx_string().unwrap_or_default();
        // Read orientation mode: 0=planar, 1=spherical, 2=cylindrical, 3=reflection
        let mode = if r.remaining() >= 4 { r.read_u32().unwrap_or(0) as u8 } else { 0 };
        log(&format!("  UV Generator: \"{}\" mode={}", name, mode));

        // Store on the last model resource (UV generators follow their parent resource)
        if let Some((_name, res)) = self.scene.model_resources.iter_mut().last() {
            res.uv_gen_mode = Some(mode);
        }
        Ok(())
    }

    fn parse_skeleton_modifier(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let _name = r.read_ifx_string()?;
        Ok(())
    }

    /// Parse generic modifier param blocks (MRM, Physics, etc.)
    /// These have a name + variable data that we log but don't process.
    fn parse_modifier_param(&mut self, r: &mut W3dBlockReader, kind: &str) -> Result<(), String> {
        let name = if r.remaining() >= 2 {
            r.read_ifx_string().unwrap_or_default()
        } else { String::new() };
        log(&format!("  {} modifier: \"{}\" ({} bytes)", kind, name, r.remaining()));
        Ok(())
    }

    /// Parse subdivision surface block: stores depth/tension/error parameters.
    fn parse_subdiv_surface(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = if r.remaining() >= 2 {
            r.read_ifx_string().unwrap_or_default()
        } else { String::new() };
        // Read SDS parameters if available
        let depth = if r.remaining() >= 4 { r.read_u32().unwrap_or(1) } else { 1 };
        let tension = if r.remaining() >= 4 { r.read_f32().unwrap_or(0.0) } else { 0.0 };
        let error = if r.remaining() >= 4 { r.read_f32().unwrap_or(0.0) } else { 0.0 };
        log(&format!("  Subdiv Surface: \"{}\" depth={} tension={:.2} error={:.2}", name, depth, tension, error));
        Ok(())
    }

    /// Parse inker modifier: outline parameters for ShaderInker.
    fn parse_inker_modifier(&mut self, r: &mut W3dBlockReader) -> Result<(), String> {
        let name = if r.remaining() >= 2 {
            r.read_ifx_string().unwrap_or_default()
        } else { String::new() };
        // Read inker parameters
        let line_width = if r.remaining() >= 4 { r.read_f32().unwrap_or(1.0) } else { 1.0 };
        log(&format!("  Inker Modifier: \"{}\" lineWidth={:.2} ({} bytes remaining)", name, line_width, r.remaining()));
        Ok(())
    }

    /// Parse extra texcoord blocks (ORIG_TEX_COORDS, TEX_COORDS).
    fn parse_extra_texcoords(&mut self, r: &mut W3dBlockReader, kind: &str) -> Result<(), String> {
        let name = if r.remaining() >= 2 {
            r.read_ifx_string().unwrap_or_default()
        } else { String::new() };
        log(&format!("  {}: \"{}\" ({} bytes)", kind, name, r.remaining()));
        Ok(())
    }

    // ─── Helpers ───

    fn read_u32_le(&mut self) -> u32 {
        if self.pos + 4 > self.data.len() { return 0; }
        let v = u32::from_le_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        v
    }
}
