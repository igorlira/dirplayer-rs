use core::fmt;
use std::collections::HashMap;
use std::fmt::Formatter;

use log::{debug, warn};

use crate::CastMemberRef;

use super::{
    bitmap::{
        bitmap::{decode_jpeg_bitmap, decompress_bitmap, Bitmap, BuiltInPalette, PaletteRef},
        manager::{BitmapManager, BitmapRef},
    },
    score::Score,
    sprite::ColorRef,
    ScriptError,
};
use crate::director::{
    chunks::{cast_member::CastMemberDef, score::{ScoreChunk, ScoreChunkHeader, ScoreFrameData}, xmedia::PfrFont, xmedia::XMediaChunk, sound::SoundChunk, Chunk, cast_member::CastMemberChunk},
    enums::{
        BitmapInfo, FilmLoopInfo, FontInfo, MemberType, ScriptType, ShapeInfo, Shockwave3dInfo, TextMemberData, SoundInfo, FieldInfo, TextInfo,
    },
    lingo::script::ScriptContext,
};
use crate::player::handlers::datum_handlers::cast_member::font::{StyledSpan, TextAlignment};

#[derive(Clone)]
pub struct CastMember {
    pub number: u32,
    pub name: String,
    pub comments: String,
    pub member_type: CastMemberType,
    pub color: ColorRef,
    pub bg_color: ColorRef,
}

#[derive(Clone)]
pub enum Media {
    Field(FieldMember),
    Bitmap(Bitmap),
    Palette(PaletteMember)
}

#[derive(Clone, Default)]
pub struct FieldMember {
    pub text: String,
    pub alignment: String,
    pub word_wrap: bool,
    pub font: String,
    pub font_style: String,
    pub font_size: u16,
    pub font_id: Option<u16>, // STXT font ID for lookup by ID
    pub text_height: u16,  // Text area height from FieldInfo (for dimension calculations)
    pub fixed_line_space: u16,  // Line spacing for text rendering
    pub top_spacing: i16,
    pub box_type: String,
    pub anti_alias: bool,
    pub width: u16,
    pub height: u16,  // Field member height from FieldInfo
    pub rect_left: i16,   // Initial rect from FieldInfo
    pub rect_top: i16,
    pub rect_right: i16,
    pub rect_bottom: i16,
    pub auto_tab: bool, // Tabbing order depends on sprite number order, not position on the Stage.
    pub editable: bool,
    pub border: u16,
    pub margin: u16,
    pub box_drop_shadow: u16,
    pub drop_shadow: u16,
    pub scroll_top: u16,
    pub hilite: bool,
    pub fore_color: Option<ColorRef>,  // From STXT formatting run color (>> 8)
    pub back_color: Option<ColorRef>,  // From FieldInfo bg RGB (& 0xff)
}

#[derive(Clone, Debug, PartialEq)]
pub enum ButtonType {
    PushButton = 0,
    CheckBox = 1,
    RadioButton = 2,
}

impl ButtonType {
    pub fn from_raw(value: u16) -> ButtonType {
        match value {
            1 => ButtonType::CheckBox,
            2 => ButtonType::RadioButton,
            _ => ButtonType::PushButton,
        }
    }

    pub fn symbol_string(&self) -> &str {
        match self {
            ButtonType::PushButton => "pushButton",
            ButtonType::CheckBox => "checkBox",
            ButtonType::RadioButton => "radioButton",
        }
    }
}

#[derive(Clone)]
pub struct ButtonMember {
    pub field: FieldMember,
    pub button_type: ButtonType,
    pub hilite: bool,
    pub script_id: u32,
    pub member_script_ref: Option<CastMemberRef>,
}

/// A tab stop definition for text members.
/// Director supports #left, #center, and #right tab types.
#[derive(Clone, Debug)]
pub struct TabStop {
    pub tab_type: String,   // "left", "center", or "right"
    pub position: i32,      // pixel position from left edge
}

#[derive(Clone)]
pub struct TextMember {
    pub text: String,
    pub html_source: String,  // Original HTML string when set via html property
    pub rtf_source: String,   // Original RTF string when set via RTF property
    pub alignment: String,
    pub box_type: String,
    pub word_wrap: bool,
    pub anti_alias: bool,
    pub font: String,
    pub font_style: Vec<String>,
    pub font_size: u16,
    pub fixed_line_space: u16,
    pub top_spacing: i16,
    pub bottom_spacing: i16,
    pub width: u16,
    pub height: u16,
    pub char_spacing: i32,
    pub tab_stops: Vec<TabStop>,
    pub html_styled_spans: Vec<StyledSpan>,
    pub info: Option<TextInfo>,
    /// Embedded 3D world for Director's "3D Text" feature (text extrusion).
    /// Lazily initialized when 3D methods (.model(), .camera(), etc.) are called.
    pub w3d: Option<Box<Shockwave3dMember>>,
}

pub struct PfrBitmap {
    pub bitmap_ref: BitmapRef,
    pub char_width: u16,
    pub char_height: u16,
    pub grid_columns: u8,
    pub grid_rows: u8,
    pub char_widths: Option<Vec<u16>>,
    pub first_char: u8,
}

impl CastMember {
    pub fn new(number: u32, member_type: CastMemberType) -> CastMember {
        CastMember {
            number,
            name: "".to_string(),
            comments: String::new(),
            member_type,
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }
}

impl FieldMember {
    pub fn new() -> FieldMember {
        FieldMember {
            text: "".to_string(),
            alignment: "left".to_string(),
            word_wrap: true,
            font: "Arial".to_string(),
            font_style: "plain".to_string(),
            font_size: 12,
            font_id: None,
            text_height: 100,
            fixed_line_space: 0,
            top_spacing: 0,
            box_type: "adjust".to_string(),
            anti_alias: false,
            width: 100,
            height: 100,
            rect_left: 0,
            rect_top: 0,
            rect_right: 100,
            rect_bottom: 100,
            auto_tab: false,
            editable: false,
            border: 0,
            margin: 0,
            box_drop_shadow: 0,
            drop_shadow: 0,
            scroll_top: 0,
            hilite: false,
            fore_color: None,
            back_color: None,
        }
    }

    pub fn from_field_info(field_info: &FieldInfo) -> FieldMember {
        let (bg_r, bg_g, bg_b) = field_info.bg_color_rgb();
        // bgpal all zeros = "no background color set" (transparent), not black
        let back_color = if field_info.bgpal_r == 0 && field_info.bgpal_g == 0 && field_info.bgpal_b == 0 {
            None
        } else {
            Some(ColorRef::Rgb(bg_r, bg_g, bg_b))
        };
        FieldMember {
            text: "".to_string(),
            alignment: field_info.alignment_str(),
            word_wrap: field_info.wordwrap(),
            font: field_info.font_name().to_string(),
            font_style: "plain".to_string(),
            font_size: 12,
            font_id: None,
            text_height: field_info.text_height,  // Text area height for dimension calculations
            fixed_line_space: 0,  // Use default line spacing for text rendering
            top_spacing: field_info.scroll as i16,
            box_type: field_info.box_type_str(),
            anti_alias: false,
            width: field_info.width(),  // Calculated from rect
            height: (field_info.text_height + 2 * field_info.border as u16 + 2 * field_info.margin as u16),  // Member height: text_height + borders + margins
            rect_left: field_info.rect_left,
            rect_top: field_info.rect_top,
            rect_right: field_info.rect_right,
            rect_bottom: field_info.rect_bottom,
            auto_tab: field_info.auto_tab(),
            editable: field_info.editable(),
            border: field_info.border as u16,
            margin: field_info.margin as u16,
            box_drop_shadow: field_info.box_drop_shadow as u16,
            drop_shadow: field_info.text_shadow as u16,
            scroll_top: field_info.scroll,
            hilite: false,
            fore_color: None, // Set later from STXT formatting run
            back_color,
        }
    }
}

impl TextMember {
    pub fn new() -> TextMember {
        TextMember {
            text: "".to_string(),
            html_source: String::new(),
            rtf_source: String::new(),
            alignment: "left".to_string(),
            word_wrap: true,
            font: "Arial".to_string(),
            font_style: vec!["plain".to_string()],
            font_size: 12,
            fixed_line_space: 0,
            top_spacing: 0,
            bottom_spacing: 0,
            box_type: "adjust".to_string(),
            anti_alias: false,
            width: 100,
            height: 20,
            char_spacing: 0,
            tab_stops: Vec::new(),
            html_styled_spans: Vec::new(),
            info: None,
            w3d: None,
        }
    }

    /// Ensure the embedded 3D world is initialized for 3D text operations.
    /// Uses TextInfo's 3TEX section for camera, lights, and material colors.
    pub fn ensure_w3d(&mut self) {
        if self.w3d.is_some() {
            return;
        }
        use crate::director::chunks::w3d::types::*;
        use crate::director::enums::TextInfo;
        let mut scene = CastMember::create_empty_w3d_scene();
        let ti = self.info.as_ref();

        // Use TextInfo 3TEX colors for lighting, fall back to text foreground color
        let (cr, cg, cb) = self.html_styled_spans.first()
            .and_then(|s| s.style.color)
            .map(|c| (
                ((c >> 16) & 0xFF) as u8,
                ((c >> 8) & 0xFF) as u8,
                (c & 0xFF) as u8,
            ))
            .unwrap_or((255, 255, 255));
        let (dir_r, dir_g, dir_b) = ti
            .map(|i| TextInfo::color_to_rgb(i.directional_color))
            .unwrap_or((cr, cg, cb));
        let (amb_r, amb_g, amb_b) = ti
            .map(|i| TextInfo::color_to_rgb(i.ambient_color))
            .unwrap_or((64, 64, 64));
        let (spec_r, spec_g, spec_b) = ti
            .map(|i| TextInfo::color_to_rgb(i.specular_color))
            .unwrap_or((34, 34, 34));
        let reflectivity = ti.map(|i| i.reflectivity as f32 / 100.0).unwrap_or(0.3);

        // Set material from TextInfo colors
        scene.materials.push(W3dMaterial {
            name: "TextMaterial".to_string(),
            diffuse: [0.0, 0.0, 0.0, 1.0], // Director text3D defaults diffuseColor to #000000
            ambient: [amb_r as f32 / 255.0, amb_g as f32 / 255.0, amb_b as f32 / 255.0, 1.0],
            emissive: [0.0, 0.0, 0.0, 1.0],
            specular: [spec_r as f32 / 255.0, spec_g as f32 / 255.0, spec_b as f32 / 255.0, 1.0],
            reflectivity,
            opacity: 1.0,
            shininess: 50.0,
        });
        if let Some(shader) = scene.shaders.first_mut() {
            shader.material_name = "TextMaterial".to_string();
        }

        // Update directional light color from TextInfo
        if let Some(light) = scene.lights.iter_mut().find(|l| l.name == "DefaultDirectional") {
            light.color = [dir_r as f32 / 255.0, dir_g as f32 / 255.0, dir_b as f32 / 255.0];
        }
        if let Some(light) = scene.lights.iter_mut().find(|l| l.name == "DefaultAmbient") {
            light.color = [amb_r as f32 / 255.0, amb_g as f32 / 255.0, amb_b as f32 / 255.0];
        }

        // Apply directionalPreset to light node transform
        if let Some(ti) = ti {
            if ti.directional_preset > 0 && ti.directional_preset <= 9 {
                if let Some(light_node) = scene.nodes.iter_mut().find(|n| n.name == "DefaultDirectional") {
                    light_node.transform = Self::directional_preset_to_transform(ti.directional_preset);
                }
            }
        }

        // Set camera position from TextInfo
        let cam_pos: Option<(f32, f32, f32)> = ti
            .map(|i| (i.camera_position_x, i.camera_position_y, i.camera_position_z))
            .filter(|&(x, y, z)| x != 0.0 || y != 0.0 || z != 0.0);
        let cam_rot: Option<(f32, f32, f32)> = ti
            .map(|i| (i.camera_rotation_x, i.camera_rotation_y, i.camera_rotation_z));
        if let Some((px, py, pz)) = cam_pos {
            // Override DefaultView camera transform with TextInfo values
            if let Some(cam_node) = scene.nodes.iter_mut().find(|n| n.name == "DefaultView") {
                // Build transform from position (rotation applied if non-zero)
                let (rx, ry, rz) = cam_rot.unwrap_or((0.0, 0.0, 0.0));
                let rx_rad = (-rx as f64).to_radians();
                let ry_rad = (-ry as f64).to_radians();
                let rz_rad = (-rz as f64).to_radians();
                let (sx, cx) = (rx_rad.sin(), rx_rad.cos());
                let (sy, cy) = (ry_rad.sin(), ry_rad.cos());
                let (sz, cz) = (rz_rad.sin(), rz_rad.cos());
                cam_node.transform = [
                    (cy*cz) as f32, (cy*sz) as f32, (-sy) as f32, 0.0,
                    (sx*sy*cz - cx*sz) as f32, (sx*sy*sz + cx*cz) as f32, (sx*cy) as f32, 0.0,
                    (cx*sy*cz + sx*sz) as f32, (cx*sy*sz - sx*cz) as f32, (cx*cy) as f32, 0.0,
                    px, py, pz, 1.0,
                ];
                // Text3D is rendered into a dynamically sized sprite/FBO. Using the
                // static empty-world 640x480 viewport here distorts the projection
                // and makes the text appear much closer than in Director.
                cam_node.screen_width = 0;
                cam_node.screen_height = 0;
            }
        }

        // Model resource for extruded text — mesh populated by ensure_text3d()
        scene.model_resources.insert("Text".to_string(), ModelResourceInfo {
            name: "Text".to_string(),
            ..Default::default()
        });
        scene.nodes.push(W3dNode {
            name: "Text".to_string(),
            node_type: W3dNodeType::Model,
            parent_name: "World".to_string(),
            resource_name: "Text".to_string(),
            model_resource_name: "Text".to_string(),
            shader_name: "DefaultShader".to_string(),
            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
            screen_width: 640, screen_height: 480,
        });

        let info = Shockwave3dInfo {
            loops: false,
            duration: 0,
            direct_to_stage: ti.map_or(false, |i| i.direct_to_stage),
            animation_enabled: ti.map_or(false, |i| i.display_mode == 1), // display_mode 1 = #mode3d
            preload: ti.map_or(false, |i| i.save_bitmap),
            reg_point: ti.map_or((0, 0), |i| (i.reg_x, i.reg_y)),
            default_rect: (0, 0, self.width as i32, self.height as i32),
            camera_position: cam_pos,
            camera_rotation: cam_rot.filter(|&(x, y, z)| x != 0.0 || y != 0.0 || z != 0.0),
            bg_color: None, // TODO: find bgColor field in TextInfo
            ambient_color: ti.map(|i| {
                let (r, g, b) = crate::director::enums::TextInfo::color_to_rgb(i.ambient_color);
                (r, g, b)
            }),
        };
        let rc_scene = std::rc::Rc::new(scene);
        let runtime_state = Shockwave3dRuntimeState::from_info(&info, Some(&rc_scene));
        self.w3d = Some(Box::new(Shockwave3dMember {
            info,
            w3d_data: Vec::new(),
            source_scene: Some(rc_scene.clone()),
            parsed_scene: Some(rc_scene),
            runtime_state,
            converted_from_text: false,
            text3d_state: ti.map(|i| Text3dState {
                tunnel_depth: i.tunnel_depth.max(1) as f32,
                smoothness: i.smoothness,
                bevel_depth: i.bevel_depth as f32,
                bevel_type: i.bevel_type,
                display_face: i.display_face,
                display_mode: i.display_mode,
                diffuse_color: (0, 0, 0),
            }),
            text3d_source: None,
        }));
    }

    /// Build a rotation matrix for the DefaultDirectional light node
    /// from a TextInfo directionalPreset value (1-9).
    ///
    /// The mesh front-face normal is (0,0,-1) and edge normals are inverted
    /// (top edge = (0,-1,0), etc.) because of face winding conventions.
    /// L (surface-to-light) must have matching signs for dot(N,L)>0.
    fn directional_preset_to_transform(preset: u32) -> [f32; 16] {
        // L direction: x component from left/right, y from top/bottom, z always -1 for front face
        let (lx, ly): (f32, f32) = match preset {
            1 => (-1.0, -1.0), // topLeft
            2 => ( 0.0, -1.0), // topCenter
            3 => ( 1.0, -1.0), // topRight
            4 => (-1.0,  0.0), // middleLeft
            5 => ( 0.0,  0.0), // middleCenter
            6 => ( 1.0,  0.0), // middleRight
            7 => (-1.0,  1.0), // bottomLeft
            8 => ( 0.0,  1.0), // bottomCenter
            9 => ( 1.0,  1.0), // bottomRight
            _ => return [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
        };
        let lz: f32 = -1.0;
        let len = (lx * lx + ly * ly + lz * lz).sqrt();
        let l = [lx / len, ly / len, lz / len];
        // Z axis of transform = -L (shader extracts -Z as light direction)
        let z = [-l[0], -l[1], -l[2]];
        // Build orthonormal basis: X = normalize(up × Z), Y = Z × X
        let up = if z[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let mut x = [
            up[1] * z[2] - up[2] * z[1],
            up[2] * z[0] - up[0] * z[2],
            up[0] * z[1] - up[1] * z[0],
        ];
        let xlen = (x[0] * x[0] + x[1] * x[1] + x[2] * x[2]).sqrt();
        x = [x[0] / xlen, x[1] / xlen, x[2] / xlen];
        let y = [
            z[1] * x[2] - z[2] * x[1],
            z[2] * x[0] - z[0] * x[2],
            z[0] * x[1] - z[1] * x[0],
        ];
        [
            x[0], x[1], x[2], 0.0,
            y[0], y[1], y[2], 0.0,
            z[0], z[1], z[2], 0.0,
            0.0,  0.0,  0.0,  1.0,
        ]
    }

    pub fn has_html_styling(&self) -> bool {
        !self.html_styled_spans.is_empty()
    }

    pub fn get_text_content(&self) -> &str {
        if self.has_html_styling() {
            // Extract plain text from HTML spans
            &self.text
        } else {
            &self.text
        }
    }
}

#[derive(Clone)]
pub struct ScriptMember {
    pub script_id: u32,
    pub script_type: ScriptType,
    pub name: String,
}

#[derive(Clone, Default)]
pub struct BitmapMember {
    pub image_ref: BitmapRef,
    pub reg_point: (i16, i16),
    pub script_id: u32,
    pub member_script_ref: Option<CastMemberRef>,
    pub info: BitmapInfo,
}

#[derive(Clone, Debug)]
pub struct PaletteMember {
    pub colors: Vec<(u8, u8, u8)>,
}

#[derive(Clone, Debug)]
pub struct VectorShapeVertex {
    pub x: f32,
    pub y: f32,
    pub handle1_x: f32,  // outgoing control point offset
    pub handle1_y: f32,
    pub handle2_x: f32,  // incoming control point offset
    pub handle2_y: f32,
}

#[derive(Clone, Debug)]
pub struct VectorShapeMember {
    pub stroke_color: (u8, u8, u8),
    pub fill_color: (u8, u8, u8),
    pub bg_color: (u8, u8, u8),
    pub end_color: (u8, u8, u8),
    pub stroke_width: f32,
    pub fill_mode: u32,     // 0=none, 1=solid, 2=gradient
    pub closed: bool,
    pub vertices: Vec<VectorShapeVertex>,
    /// Bounding box computed from vertices + control points + stroke padding
    pub bbox_left: f32,
    pub bbox_top: f32,
    pub bbox_right: f32,
    pub bbox_bottom: f32,
}

impl VectorShapeMember {
    pub fn width(&self) -> f32 { self.bbox_right - self.bbox_left }
    pub fn height(&self) -> f32 { self.bbox_bottom - self.bbox_top }
}

#[derive(Clone)]
pub struct ShapeMember {
    pub shape_info: ShapeInfo,
    pub script_id: u32,
    pub member_script_ref: Option<CastMemberRef>,
}

impl PaletteMember {
    pub fn new() -> PaletteMember {
        PaletteMember {
            colors: vec![(0, 0, 0); 256],
        }
    }
}

#[derive(Clone)]
pub struct FilmLoopMember {
    pub info: FilmLoopInfo,
    pub score_chunk: ScoreChunk,
    pub score: Score,
    pub current_frame: u32,
    /// The bounding rectangle encompassing all sprites in the filmloop.
    /// Used to translate sprite coordinates when rendering.
    pub initial_rect: super::geometry::IntRect,
}

#[derive(Clone)]
pub struct SoundMember {
    pub info: SoundInfo,
    pub sound: SoundChunk,
}

#[derive(Clone)]
pub struct FlashMember {
    pub data: Vec<u8>,
    pub reg_point: (i16, i16),
    pub flash_info: Option<crate::director::enums::FlashInfo>,
}

#[derive(Clone, Debug)]
pub struct Text3dState {
    pub tunnel_depth: f32,
    pub smoothness: u32,
    pub bevel_depth: f32,
    pub bevel_type: u32,
    pub display_face: i32,
    pub display_mode: u32,
    pub diffuse_color: (u8, u8, u8), // Director defaults to #000000
}

#[derive(Clone, Debug)]
pub struct Text3dSource {
    pub spans: Vec<StyledSpan>,
    pub font_size: u16,
    pub width: u16,
    pub height: u16,
    pub alignment: String,
    pub word_wrap: bool,
    pub fixed_line_space: u16,
    pub top_spacing: i16,
    pub bottom_spacing: i16,
    pub tab_stops: Vec<TabStop>,
    pub native_alpha_mesh: bool,
}

#[derive(Clone, Debug)]
pub struct Shockwave3dMember {
    pub info: Shockwave3dInfo,
    pub w3d_data: Vec<u8>,
    pub source_scene: Option<std::rc::Rc<crate::director::chunks::w3d::types::W3dScene>>,
    pub parsed_scene: Option<std::rc::Rc<crate::director::chunks::w3d::types::W3dScene>>,
    pub runtime_state: Shockwave3dRuntimeState,
    /// True if this member was converted from a Text member (3D Text feature).
    /// Used to report member.type as #text instead of #shockwave3d.
    pub converted_from_text: bool,
    /// Live Text3D properties retained after Text -> Shockwave3d conversion.
    pub text3d_state: Option<Text3dState>,
    /// Source data retained so native-font Text3D meshes can be rebuilt.
    pub text3d_source: Option<Text3dSource>,
}

impl Shockwave3dMember {
    /// Get mutable access to the parsed scene (uses Rc::make_mut for copy-on-write)
    pub fn scene_mut(&mut self) -> Option<&mut crate::director::chunks::w3d::types::W3dScene> {
        self.parsed_scene.as_mut().map(|rc| std::rc::Rc::make_mut(rc))
    }
}

#[derive(Clone, Debug)]
pub struct QueuedMotion {
    pub name: String,
    pub looped: bool,
    pub start_time: f32,   // seconds
    pub end_time: f32,     // seconds, -1.0 = full duration
    pub scale: f32,
    pub offset: f32,       // seconds, -1.0 = #synchronized
}

/// Mutable runtime state for a Shockwave 3D member (animation, transforms, etc.)
#[derive(Clone, Debug, Default)]
pub struct Shockwave3dRuntimeState {
    // ─── Animation ───
    pub animation_time: f32,
    pub animation_playing: bool,
    pub current_motion: Option<String>,
    pub play_rate: f32,
    pub animation_loop: bool,
    pub motion_queue: Vec<QueuedMotion>,
    pub animation_start_time: f32,
    pub animation_end_time: f32,
    pub animation_blend_time: f32,
    pub root_lock: bool,
    /// Per-motion playrate scale (arg 5 of play()), multiplied with play_rate
    pub animation_scale: f32,
    /// Whether the current non-looping motion has ended
    pub motion_ended: bool,
    /// Previous motion for crossfade blending
    pub previous_motion: Option<String>,
    pub blend_weight: f32,       // 0.0 = all previous, 1.0 = all current
    pub blend_duration: f32,     // total blend time in seconds
    pub blend_elapsed: f32,      // time spent blending

    // ─── Per-node overrides (keyed by node name) ───
    /// Transform overrides for nodes (set via Lingo) — used by renderer
    pub node_transforms: std::collections::HashMap<String, [f32; 16]>,
    /// Persistent Transform3d DatumRefs per node — returned by .transform getter
    /// so that chained mutations (model.transform.position = v) persist
    pub node_transform_datums: std::collections::HashMap<String, crate::player::DatumRef>,
    /// Director-friendly Euler readback hints for nodes oriented via pointAt().
    pub node_rotation_hints: std::collections::HashMap<String, [f64; 3]>,
    /// Visibility overrides for nodes: 0=#none, 1=#front, 2=#back, 3=#both
    pub node_visibility: std::collections::HashMap<String, u8>,
    /// Shader overrides for nodes: model_name → (mesh_index → shader_name)
    /// mesh_index is 0-based; index 0 is also the whole-model fallback
    pub node_shaders: std::collections::HashMap<String, std::collections::HashMap<usize, String>>,

    // ─── World state ───
    pub background_color: Option<(u8, u8, u8)>,
    pub ambient_color: Option<(f32, f32, f32)>,

    // ─── Fog ───
    pub fog_enabled: bool,
    pub fog_near: f32,
    pub fog_far: f32,
    pub fog_color: (f32, f32, f32),
    pub fog_mode: u8, // 0=linear, 1=exp, 2=exp2

    // ─── Post-processing effects ───
    pub bloom_enabled: bool,
    pub bloom_threshold: f32,
    pub bloom_intensity: f32,

    // ─── Camera ───
    /// Per-camera projection mode: 0=perspective (default), 1=orthographic
    pub camera_projection_mode: std::collections::HashMap<String, u8>,
    /// Per-camera ortho height (world units visible vertically)
    pub camera_ortho_height: std::collections::HashMap<String, f32>,
    /// Render-to-texture requests: camera_name → target_texture_name
    /// When set, the next render from this camera writes to the named texture instead of the main FBO
    pub render_targets: std::collections::HashMap<String, String>,

    // ─── Particle systems ───
    pub particles: std::collections::HashMap<String, ParticleSystemState>,

    // ─── Shader persistent lists ───
    /// Per-shader persistent textureList DatumRefs: shader_name -> DatumRef to list
    pub shader_texture_lists: std::collections::HashMap<String, crate::player::DatumRef>,
    /// Per-shader persistent textureTransformList DatumRefs: shader_name -> DatumRef to list of Transform3d
    pub shader_texture_transform_lists: std::collections::HashMap<String, crate::player::DatumRef>,

    // ─── MeshDeform state ───
    /// Per-model mesh deform data: model_name -> list of mesh texture layers
    /// Each mesh has a Vec of texture layers, each layer has texture coordinates
    pub mesh_deform: std::collections::HashMap<String, MeshDeformState>,

    // ─── Particle emitter state ───
    /// Per-resource emitter state: resource_name -> emitter properties
    pub emitters: std::collections::HashMap<String, EmitterState>,

    // ─── Level of Detail (LOD) state ───
    pub lod_state: std::collections::HashMap<String, LodState>,

    // ─── Subdivision Surface (SDS) state ───
    pub sds_state: std::collections::HashMap<String, SdsState>,

    // ─── Reset tracking ───
    pub world_reset: bool,

    // ─── Detached nodes (parent set to VOID) ───
    pub detached_nodes: std::collections::HashSet<String>,

    // ─── Camera properties ───
    /// Per-camera rootNode: camera_name -> node_name (limits which subtree to render)
    pub camera_root_nodes: std::collections::HashMap<String, String>,
    /// Per-camera colorBuffer.clearAtRender: camera_name -> bool
    pub camera_clear_at_render: std::collections::HashMap<String, bool>,

    // ─── Camera overlays/backdrops ───
    /// Per-camera overlay list: camera_name -> Vec<CameraOverlay>
    pub camera_overlays: std::collections::HashMap<String, Vec<CameraOverlay>>,
    pub camera_backdrops: std::collections::HashMap<String, Vec<CameraOverlay>>,

    // ─── Mesh build data (for newMesh() → build() workflow) ───
    /// Per-model-resource mesh build data: resource_name -> MeshBuildData
    pub mesh_build_data: std::collections::HashMap<String, MeshBuildData>,
}

/// Stores intermediate data for newMesh() model resources before build() is called.
/// Holds vertexList, textureCoordinateList, colorList, normalList, and the
/// generateNormals style.
#[derive(Clone, Debug, Default)]
pub struct MeshBuildData {
    pub vertex_list: Vec<[f32; 3]>,
    pub texture_coordinate_list: Vec<[f32; 2]>,
    pub color_list: Vec<(u8, u8, u8)>,
    pub normal_list: Vec<[f32; 3]>,
    /// #flat = 0, #smooth = 1, None = not called
    pub generate_normals_style: Option<u8>,
}

#[derive(Clone, Debug)]
pub struct CameraOverlay {
    pub source_texture: String,
    pub source_texture_lower: String, // pre-lowercased for GPU texture lookup
    pub loc: [f64; 2],
    pub rotation: f64,
    pub blend: f64,
    pub scale: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub reg_point: [f64; 2],
    pub shader_name: String,
}

impl Default for CameraOverlay {
    fn default() -> Self {
        Self {
            source_texture: String::new(),
            source_texture_lower: String::new(),
            loc: [0.0, 0.0],
            rotation: 0.0,
            blend: 100.0,
            scale: 1.0,
            scale_x: 1.0,
            scale_y: 1.0,
            reg_point: [0.0, 0.0],
            shader_name: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub struct LodState {
    pub level: i32,
    pub auto_mode: bool,
    pub bias: f32,
}

impl Default for LodState {
    fn default() -> Self {
        Self { level: 100, auto_mode: true, bias: 100.0 }
    }
}

#[derive(Clone, Debug)]
pub struct SdsState {
    pub depth: i32,
    pub tension: f32,
    pub error: f32,
    pub enabled: bool,
}

impl Default for SdsState {
    fn default() -> Self {
        Self { depth: 1, tension: 0.0, error: 0.0, enabled: true }
    }
}

/// Runtime mesh deform state for a model
#[derive(Clone, Debug, Default)]
pub struct MeshDeformState {
    /// Per-mesh texture layer data
    pub meshes: Vec<MeshDeformMesh>,
}

#[derive(Clone, Debug, Default)]
pub struct MeshDeformMesh {
    /// Texture layers for this mesh
    pub texture_layers: Vec<MeshDeformTextureLayer>,
    /// Persistent DatumRef to the textureLayer list so add() persists
    pub texture_layer_datum_ref: Option<crate::player::DatumRef>,
}

#[derive(Clone, Debug, Default)]
pub struct MeshDeformTextureLayer {
    pub texture_coordinate_list: Vec<[f32; 2]>,
}

/// Emitter properties for a particle model resource
#[derive(Clone, Debug)]
pub struct EmitterState {
    pub num_particles: i32,
    pub mode: String,       // "burst" or "stream"
    pub is_loop: bool,
    pub direction: [f64; 3],
    pub region: [f64; 3],
    pub distribution: String, // "linear", "gaussian"
    pub angle: f64,
    pub min_speed: f64,
    pub max_speed: f64,
    pub path_strength: f64,
}

impl Default for EmitterState {
    fn default() -> Self {
        Self {
            num_particles: 100,
            mode: "burst".to_string(),
            is_loop: true,
            direction: [0.0, 1.0, 0.0],
            region: [0.0, 0.0, 0.0],
            distribution: "linear".to_string(),
            angle: 180.0,
            min_speed: 1.0,
            max_speed: 1.0,
            path_strength: 0.0,
        }
    }
}

/// Runtime state for a particle system
#[derive(Clone, Debug)]
pub struct ParticleSystemState {
    pub positions: Vec<[f32; 3]>,
    pub velocities: Vec<[f32; 3]>,
    pub ages: Vec<f32>,
    pub alive: Vec<bool>,
    pub max_particles: usize,
    pub lifetime: f32,
    pub gravity: [f32; 3],
    pub wind: [f32; 3],
    pub drag: f32,
    pub initial_speed: f32,
    pub speed_range: f32,  // random speed variation
    pub direction: [f32; 3],
    pub emitter_position: [f32; 3],
    pub emitter_shape: u8,      // 0=point, 1=line, 2=plane, 3=sphere, 4=cube, 5=cylinder
    pub emitter_size: [f32; 3], // dimensions of emitter shape
    pub angle_range: f32,       // emission cone angle (0 = parallel, PI = hemisphere)
    pub particle_size: f32,
}

impl Shockwave3dRuntimeState {
    /// Create runtime state initialized with camera data from the 3DPR info
    /// and optionally auto-start animation if animationEnabled is set.
    pub fn from_info(info: &Shockwave3dInfo, scene: Option<&crate::director::chunks::w3d::types::W3dScene>) -> Self {
        let mut state = Self {
            play_rate: 1.0,
            animation_scale: 1.0,
            animation_end_time: -1.0,
            ..Default::default()
        };
        // Seed camera transform from 3DPR camera position/rotation
        if let Some((px, py, pz)) = info.camera_position {
            let (rx, ry, rz) = info.camera_rotation.unwrap_or((0.0, 0.0, 0.0));
            // Build camera transform from position + Euler rotation (degrees)
            // Director uses negative rotation convention, order: Z * Y * X
            let rx_rad = (-rx).to_radians();
            let ry_rad = (-ry).to_radians();
            let rz_rad = (-rz).to_radians();
            let (sx, cx) = (rx_rad.sin(), rx_rad.cos());
            let (sy, cy) = (ry_rad.sin(), ry_rad.cos());
            let (sz, cz) = (rz_rad.sin(), rz_rad.cos());

            // Rotation = Rz * Ry * Rx (column-major)
            let m = [
                cy*cz,              cy*sz,              -sy,     0.0,
                sx*sy*cz - cx*sz,   sx*sy*sz + cx*cz,   sx*cy,  0.0,
                cx*sy*cz + sx*sz,   cx*sy*sz - sx*cz,   cx*cy,  0.0,
                px,                 py,                 pz,      1.0,
            ];
            state.node_transforms.insert("DefaultView".to_string(), m);
            state.node_transforms.insert("defaultview".to_string(), m);
        }
        if let Some(bg) = info.bg_color {
            state.background_color = Some(bg);
        }
        // Note: animationEnabled auto-start is handled in the rendering path
        // when the sprite first appears on stage, not here at parse time.
        state
    }
}

impl Default for ParticleSystemState {
    fn default() -> Self {
        Self {
            positions: Vec::new(),
            velocities: Vec::new(),
            ages: Vec::new(),
            alive: Vec::new(),
            max_particles: 100,
            lifetime: 10.0,
            gravity: [0.0, -9.8, 0.0],
            wind: [0.0; 3],
            drag: 0.0,
            initial_speed: 1.0,
            speed_range: 0.0,
            direction: [0.0, 1.0, 0.0],
            emitter_position: [0.0; 3],
            emitter_shape: 0,
            emitter_size: [1.0; 3],
            angle_range: 0.3,
            particle_size: 1.0,
        }
    }
}

impl ParticleSystemState {
    pub fn initialize(&mut self, count: usize) {
        self.max_particles = count;
        self.positions = vec![[0.0; 3]; count];
        self.velocities = vec![[0.0; 3]; count];
        self.ages = vec![0.0; count];
        self.alive = vec![false; count];

        // Stagger initial ages
        for i in 0..count {
            self.ages[i] = (i as f32 / count as f32) * self.lifetime;
            self.alive[i] = false;
        }
    }

    pub fn update(&mut self, dt: f32) {
        for i in 0..self.max_particles {
            if i >= self.ages.len() { break; }

            self.ages[i] += dt;

            if self.ages[i] >= self.lifetime {
                // Recycle
                self.ages[i] -= self.lifetime;
                self.alive[i] = true;
                // Emitter shape offset
                let hash = (i as u32).wrapping_mul(2654435761); // Knuth hash for pseudo-random
                let r1 = (hash & 0xFF) as f32 / 255.0 - 0.5;
                let r2 = ((hash >> 8) & 0xFF) as f32 / 255.0 - 0.5;
                let r3 = ((hash >> 16) & 0xFF) as f32 / 255.0 - 0.5;
                let offset = match self.emitter_shape {
                    1 => [r1 * self.emitter_size[0], 0.0, 0.0],              // line
                    2 => [r1 * self.emitter_size[0], 0.0, r2 * self.emitter_size[2]], // plane
                    3 => {                                                      // sphere
                        let len = (r1*r1 + r2*r2 + r3*r3).sqrt().max(0.01);
                        let s = self.emitter_size[0] * ((hash & 0xFF) as f32 / 255.0);
                        [r1/len * s, r2/len * s, r3/len * s]
                    }
                    4 => [r1 * self.emitter_size[0], r2 * self.emitter_size[1], r3 * self.emitter_size[2]], // cube
                    _ => [0.0, 0.0, 0.0],                                      // point
                };
                self.positions[i] = [
                    self.emitter_position[0] + offset[0],
                    self.emitter_position[1] + offset[1],
                    self.emitter_position[2] + offset[2],
                ];
                // Direction with angle spread
                let spread = self.angle_range;
                let jx = r1 * spread;
                let jy = r2 * spread;
                let jz = r3 * spread;
                let speed = self.initial_speed + r1 * self.speed_range;
                self.velocities[i] = [
                    (self.direction[0] + jx) * speed,
                    (self.direction[1] + jy) * speed,
                    (self.direction[2] + jz) * speed,
                ];
            }

            if self.alive[i] {
                // Apply gravity
                self.velocities[i][0] += self.gravity[0] * dt;
                self.velocities[i][1] += self.gravity[1] * dt;
                self.velocities[i][2] += self.gravity[2] * dt;

                // Apply wind drag
                if self.drag > 0.0 {
                    let factor = 1.0 - self.drag * dt;
                    self.velocities[i][0] = self.velocities[i][0] * factor + self.wind[0] * self.drag * dt;
                    self.velocities[i][1] = self.velocities[i][1] * factor + self.wind[1] * self.drag * dt;
                    self.velocities[i][2] = self.velocities[i][2] * factor + self.wind[2] * self.drag * dt;
                }

                // Integrate position
                self.positions[i][0] += self.velocities[i][0] * dt;
                self.positions[i][1] += self.velocities[i][1] * dt;
                self.positions[i][2] += self.velocities[i][2] * dt;
            }
        }
    }
}

#[derive(Clone, Debug)]
pub struct FontMember {
    pub font_info: FontInfo,
    pub preview_text: String,
    pub preview_font_name: Option<String>,
    pub preview_html_spans: Vec<StyledSpan>,
    pub fixed_line_space: u16,
    pub top_spacing: i16,
    pub bitmap_ref: Option<BitmapRef>,
    pub char_width: Option<u16>,
    pub char_height: Option<u16>,
    pub grid_columns: Option<u8>,
    pub grid_rows: Option<u8>,
    pub char_widths: Option<Vec<u16>>,
    pub first_char_num: Option<u8>,
    pub alignment: TextAlignment,
    pub pfr_parsed: Option<crate::director::chunks::pfr1::types::Pfr1ParsedFont>,
    pub pfr_data: Option<Vec<u8>>,
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum CastMemberType {
    Field(FieldMember),
    Text(TextMember),
    Button(ButtonMember),
    Script(ScriptMember),
    Bitmap(BitmapMember),
    Palette(PaletteMember),
    Shape(ShapeMember),
    VectorShape(VectorShapeMember),
    FilmLoop(FilmLoopMember),
    Sound(SoundMember),
    Font(FontMember),
    Flash(FlashMember),
    Shockwave3d(Shockwave3dMember),
    Unknown,
}

#[derive(Debug, PartialEq)]
pub enum CastMemberTypeId {
    Field,
    Text,
    Button,
    Script,
    Bitmap,
    Palette,
    Shape,
    VectorShape,
    FilmLoop,
    Sound,
    Font,
    Flash,
    Shockwave3d,
    Unknown,
}

impl fmt::Debug for CastMemberType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(_) => {
                write!(f, "Field")
            }
            Self::Text(_) => {
                write!(f, "Text")
            }
            Self::Button(_) => {
                write!(f, "Button")
            }
            Self::Script(_) => {
                write!(f, "Script")
            }
            Self::Bitmap(_) => {
                write!(f, "Bitmap")
            }
            Self::Palette(_) => {
                write!(f, "Palette")
            }
            Self::Shape(_) => {
                write!(f, "Shape")
            }
            Self::VectorShape(_) => {
                write!(f, "VectorShape")
            }
            Self::FilmLoop(_) => {
                write!(f, "FilmLoop")
            }
            Self::Sound(_) => {
                write!(f, "Sound")
            }
            Self::Font(_) => {
                write!(f, "Font")
            }
            Self::Flash(_) => {
                write!(f, "Flash")
            }
            Self::Shockwave3d(_) => {
                write!(f, "Shockwave3d")
            }
            Self::Unknown => {
                write!(f, "Unknown")
            }
        }
    }
}

impl CastMemberTypeId {
    pub fn symbol_string(&self) -> Result<&str, ScriptError> {
        return match self {
            Self::Field => Ok("field"),
            Self::Text => Ok("text"),
            Self::Button => Ok("button"),
            Self::Script => Ok("script"),
            Self::Bitmap => Ok("bitmap"),
            Self::Palette => Ok("palette"),
            Self::Shape => Ok("shape"),
            Self::VectorShape => Ok("vectorShape"),
            Self::FilmLoop => Ok("filmLoop"),
            Self::Sound => Ok("sound"),
            Self::Font => Ok("font"),
            Self::Flash => Ok("flash"),
            Self::Shockwave3d => Ok("shockwave3d"),
            Self::Unknown => Ok("unknown"),
        };
    }
}

impl CastMemberType {
    pub fn member_type_id(&self) -> CastMemberTypeId {
        return match self {
            Self::Field(_) => CastMemberTypeId::Field,
            Self::Text(_) => CastMemberTypeId::Text,
            Self::Button(_) => CastMemberTypeId::Button,
            Self::Script(_) => CastMemberTypeId::Script,
            Self::Bitmap(_) => CastMemberTypeId::Bitmap,
            Self::Palette(_) => CastMemberTypeId::Palette,
            Self::Shape(_) => CastMemberTypeId::Shape,
            Self::VectorShape(_) => CastMemberTypeId::VectorShape,
            Self::FilmLoop(_) => CastMemberTypeId::FilmLoop,
            Self::Sound(_) => CastMemberTypeId::Sound,
            Self::Font(_) => CastMemberTypeId::Font,
            Self::Flash(_) => CastMemberTypeId::Flash,
            Self::Shockwave3d(_) => CastMemberTypeId::Shockwave3d,
            Self::Unknown => CastMemberTypeId::Unknown,
        };
    }

    pub fn type_string(&self) -> &str {
        return match self {
            Self::Field(_) => "field",
            Self::Text(_) => "text",
            Self::Button(_) => "button",
            Self::Script(_) => "script",
            Self::Bitmap(_) => "bitmap",
            Self::Palette(_) => "palette",
            Self::Shape(_) => "shape",
            Self::VectorShape(_) => "vectorShape",
            Self::FilmLoop(_) => "filmLoop",
            Self::Sound(_) => "sound",
            Self::Font(_) => "font",
            Self::Flash(_) => "flash",
            Self::Shockwave3d(w3d) => if w3d.converted_from_text { "text" } else { "shockwave3d" },
            _ => "unknown",
        };
    }

    #[allow(dead_code)]
    pub fn as_script(&self) -> Option<&ScriptMember> {
        return match self {
            Self::Script(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_field(&self) -> Option<&FieldMember> {
        return match self {
            Self::Field(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_field_mut(&mut self) -> Option<&mut FieldMember> {
        return match self {
            Self::Field(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_text(&self) -> Option<&TextMember> {
        return match self {
            Self::Text(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_text_mut(&mut self) -> Option<&mut TextMember> {
        return match self {
            Self::Text(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_button(&self) -> Option<&ButtonMember> {
        return match self {
            Self::Button(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_button_mut(&mut self) -> Option<&mut ButtonMember> {
        return match self {
            Self::Button(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_bitmap(&self) -> Option<&BitmapMember> {
        return match self {
            Self::Bitmap(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_bitmap_mut(&mut self) -> Option<&mut BitmapMember> {
        return match self {
            Self::Bitmap(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_palette(&self) -> Option<&PaletteMember> {
        return match self {
            Self::Palette(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_film_loop(&self) -> Option<&FilmLoopMember> {
        return match self {
            Self::FilmLoop(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_film_loop_mut(&mut self) -> Option<&mut FilmLoopMember> {
        return match self {
            Self::FilmLoop(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_sound(&self) -> Option<&SoundMember> {
        return match self {
            Self::Sound(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_font(&self) -> Option<&FontMember> {
        return match self {
            Self::Font(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_flash(&self) -> Option<&FlashMember> {
        return match self {
            Self::Flash(data) => { Some(data) }
            _ => { None }
        }
    }

    pub fn as_flash_mut(&mut self) -> Option<&mut FlashMember> {
        return match self {
            Self::Flash(data) => { Some(data) }
            _ => { None }
        }
    }

    pub fn as_shockwave3d(&self) -> Option<&Shockwave3dMember> {
        match self {
            Self::Shockwave3d(data) => Some(data),
            Self::Text(text) => text.w3d.as_deref(),
            _ => None,
        }
    }

    pub fn as_shockwave3d_mut(&mut self) -> Option<&mut Shockwave3dMember> {
        match self {
            Self::Shockwave3d(data) => Some(data),
            Self::Text(text) => text.w3d.as_deref_mut(),
            _ => None,
        }
    }
}

impl CastMember {
    fn chunk_type_name(c: &Chunk) -> &'static str {
        match c {
            Chunk::Cast(_) => "Cast",
            Chunk::CastList(_) => "CastList",
            Chunk::CastMember(_) => "CastMember",
            Chunk::CastInfo(_) => "CastInfo",
            Chunk::Config(_) => "Config",
            Chunk::InitialMap(_) => "InitialMap",
            Chunk::KeyTable(_) => "KeyTable",
            Chunk::MemoryMap(_) => "MemoryMap",
            Chunk::Script(_) => "Script",
            Chunk::ScriptContext(_) => "ScriptContext",
            Chunk::ScriptNames(_) => "ScriptNames",
            Chunk::FrameLabels(_) => "FrameLabels",
            Chunk::Score(_) => "Score",
            Chunk::ScoreOrder(_) => "ScoreOrder",
            Chunk::Text(_) => "Text",
            Chunk::Bitmap(_) => "Bitmap",
            Chunk::Palette(_) => "Palette",
            Chunk::Sound(_) => "Sound",
            Chunk::SndHeader(_) => "SndHeader",
            Chunk::SndSamples(_) => "SndSamples",
            Chunk::Media(_) => "Media",
            Chunk::XMedia(_) => "XMedia",
            Chunk::CstInfo(_) => "Cinf",
            Chunk::Effect(_) => "FXmp",
            Chunk::Thum(_) => "Thum",
            Chunk::Raw(_) => "Raw",
        }
    }

    /// Recursively searches children of a CastMemberDef for a sound chunk
    fn find_sound_chunk_in_def(def: &CastMemberDef) -> Option<SoundChunk> {
        // First check for direct sound/media chunks
        for child_opt in &def.children {
            if let Some(child) = child_opt {
                match child {
                    Chunk::Sound(s) => return Some(s.clone()),
                    Chunk::Media(m) => {
                        if !m.audio_data.is_empty() {
                            let mut sc = SoundChunk::new(m.audio_data.clone());
                            sc.set_metadata(m.sample_rate, 1, if m.is_compressed { 0 } else { 16 });
                            return Some(sc);
                        }
                    }
                    Chunk::CastMember(_) => {
                        // `CastMemberChunk` has no children, so nothing to recurse into
                        continue;
                    }
                    _ => {}
                }
            }
        }
        // Then try sndH + sndS combination
        let snd_header = def.children.iter()
            .filter_map(|c| c.as_ref())
            .find_map(|c| match c { Chunk::SndHeader(h) => Some(h), _ => None });
        let snd_samples = def.children.iter()
            .filter_map(|c| c.as_ref())
            .find_map(|c| match c { Chunk::SndSamples(d) => Some(d), _ => None });
        if let (Some(header), Some(samples)) = (snd_header, snd_samples) {
            return Some(SoundChunk::from_snd_header_and_samples(header, samples));
        }
        None
    }

    fn child_has_sound_in_def(def: &CastMemberDef) -> bool {
        let has_direct = def.children.iter().any(|c| match c {
            Some(Chunk::Sound(_)) => true,
            Some(Chunk::Media(m)) => !m.audio_data.is_empty(),
            Some(Chunk::CastMember(_)) => false,
            _ => false,
        });
        if has_direct { return true; }
        // Check for sndH + sndS
        let has_header = def.children.iter().any(|c| matches!(c, Some(Chunk::SndHeader(_))));
        let has_samples = def.children.iter().any(|c| matches!(c, Some(Chunk::SndSamples(_))));
        has_header && has_samples
    }

    /// Recursively find a SoundChunk in a Chunk (handles Media & nested CastMembers)
    fn find_sound_chunk_in_chunk(chunk: &Chunk) -> Option<SoundChunk> {
        match chunk {
            Chunk::Sound(s) => Some(s.clone()),
            Chunk::Media(m) if !m.audio_data.is_empty() => {
                let mut sc = SoundChunk::new(m.audio_data.clone());
                sc.set_metadata(m.sample_rate, 1, if m.is_compressed { 0 } else { 16 });
                Some(sc)
            }
            Chunk::CastMember(cm) => {
                // CastMemberChunk has no children; nothing to recurse
                None
            }
            _ => None,
        }
    }

    // Check if an Option<Chunk> contains sound
    fn chunk_has_sound(chunk_opt: &Option<Chunk>) -> bool {
        match chunk_opt {
            Some(c) => match c {
                Chunk::Sound(_) => true,
                Chunk::Media(m) => !m.audio_data.is_empty(),
                _ => false,
            },
            None => false,
        }
    }

    // Extract SoundChunk from an Option<Chunk>
    fn find_sound_chunk(chunk_opt: &Option<Chunk>) -> Option<SoundChunk> {
        match chunk_opt {
            Some(c) => match c {
                Chunk::Sound(s) => Some(s.clone()),
                Chunk::Media(m) => {
                    if !m.audio_data.is_empty() {
                        Some(SoundChunk::from_media(m))
                    } else {
                        None
                    }
                }
                _ => None,
            },
            None => None,
        }
    }

    /// Compute the initial bounding rectangle for a filmloop by finding the
    /// bounding box of all sprites across all frames.
    ///
    /// The coordinate system for filmloop sprites is relative to this initial_rect.
    /// When rendering, sprite positions are translated by subtracting initial_rect.left/top.
    fn compute_filmloop_initial_rect(
        frame_channel_data: &[(u32, u16, crate::director::chunks::score::ScoreFrameChannelData)],
        _reg_point: (i16, i16),
    ) -> super::geometry::IntRect {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        let mut found_any = false;

        for (_frame_idx, channel_idx, data) in frame_channel_data.iter() {
            // Skip effect channels (channels 0-5 in the raw data)
            // Real sprite channels start at index 6
            if *channel_idx < 6 {
                continue;
            }

            // Skip empty sprites (no cast member assigned)
            // Also skip sprites with cast_lib == 0 which are typically invalid/placeholder entries
            // (cast_lib 65535 is valid - it's used for internal/embedded casts)
            if data.cast_member == 0 || data.cast_lib == 0 || (data.width == 0 && data.height == 0) {
                continue;
            }

            // The sprite's position (pos_x, pos_y) is its loc (registration point location).
            // In Director, loc is where the reg point is placed.
            // Since we don't have access to cast members here, we assume CENTER registration
            // which is the default for bitmaps. This means:
            //   sprite_left = pos_x - width/2
            //   sprite_top = pos_y - height/2
            let reg_offset_x = data.width as i32 / 2;
            let reg_offset_y = data.height as i32 / 2;
            let sprite_left = data.pos_x as i32 - reg_offset_x;
            let sprite_top = data.pos_y as i32 - reg_offset_y;
            let sprite_right = sprite_left + data.width as i32;
            let sprite_bottom = sprite_top + data.height as i32;

            debug!(
                "FilmLoop initial_rect: frame {} channel {} cast {}:{} pos ({}, {}) size {}x{} -> bounds ({}, {}, {}, {})",
                _frame_idx, channel_idx, data.cast_lib, data.cast_member,
                data.pos_x, data.pos_y, data.width, data.height,
                sprite_left, sprite_top, sprite_right, sprite_bottom
            );

            if sprite_left < min_x {
                min_x = sprite_left;
            }
            if sprite_top < min_y {
                min_y = sprite_top;
            }
            if sprite_right > max_x {
                max_x = sprite_right;
            }
            if sprite_bottom > max_y {
                max_y = sprite_bottom;
            }
            found_any = true;
        }

        if !found_any {
            // No sprites found, return a default rect at origin
            debug!("FilmLoop initial_rect: no sprites found, using default (0, 0, 1, 1)");
            return super::geometry::IntRect::from(0, 0, 1, 1);
        }

        debug!(
            "FilmLoop initial_rect computed: ({}, {}, {}, {})",
            min_x, min_y, max_x, max_y
        );
        super::geometry::IntRect::from(min_x, min_y, max_x, max_y)
    }

    fn decode_bitmap_from_bitd(
        member_def: &CastMemberDef,
        bitmap_info: &BitmapInfo,
        cast_lib: u32,
        number: u32,
        bitmap_manager: &mut BitmapManager,
    ) -> BitmapRef {
        // Search all children for the first Bitmap(BITD) chunk
        // (it may not be at index 0 — other slots can be None or other chunk types)
        let bitd_chunk = member_def.children.iter()
            .find_map(|c| c.as_ref().and_then(|chunk| chunk.as_bitmap()));

        if let Some(bitd_chunk) = bitd_chunk {
            // Check if BITD contains JPEG data with a separate ALFA chunk
            let is_jpeg = bitd_chunk.data.len() >= 3
                && bitd_chunk.data[0] == 0xFF
                && bitd_chunk.data[1] == 0xD8
                && bitd_chunk.data[2] == 0xFF;

            let alfa_data: Option<&Vec<u8>> = member_def.children.iter().find_map(|c| {
                c.as_ref().and_then(|chunk| match chunk {
                    Chunk::Raw(data) if !data.is_empty() => Some(data),
                    _ => None,
                })
            });

            if is_jpeg && alfa_data.is_some() {
                // JPEG in BITD + separate ALFA chunk: use decode_jpeg_bitmap which
                // correctly combines JPEG RGB with ALFA alpha channel.
                // decode_jpeg_bitd only looks for alpha AFTER FFD9 inside the BITD data,
                // missing the separate ALFA chunk entirely.
                match decode_jpeg_bitmap(&bitd_chunk.data, bitmap_info, alfa_data) {
                    Ok(new_bitmap) => bitmap_manager.add_bitmap(new_bitmap),
                    Err(e) => {
                        warn!(
                            "Failed to decode JPEG+ALFA bitmap {}: {:?}. Using empty image.",
                            number, e
                        );
                        bitmap_manager.add_bitmap(Bitmap::new(
                            1, 1, 8, 8, 0,
                            PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
                        ))
                    }
                }
            } else {
                let decompressed =
                    decompress_bitmap(&bitd_chunk.data, bitmap_info, cast_lib, bitd_chunk.version);
                match decompressed {
                    Ok(new_bitmap) => bitmap_manager.add_bitmap(new_bitmap),
                    Err(e) => {
                        warn!(
                            "Failed to decompress bitmap {}: {:?}. Using empty image.",
                            number, e
                        );
                        bitmap_manager.add_bitmap(Bitmap::new(
                            1, 1, 8, 8, 0,
                            PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
                        ))
                    }
                }
            }
        } else {
            // No BITD chunk — try Raw chunk data as bitmap pixel data.
            // Chunk::Raw can be either ALFA data or an unrecognized chunk type
            // that contains actual bitmap pixel data, so we must try decompression.
            let raw_data = member_def.children.iter().find_map(|c| {
                c.as_ref().and_then(|chunk| match chunk {
                    Chunk::Raw(data) if !data.is_empty() => Some(data.as_slice()),
                    _ => None,
                })
            });
            if let Some(data) = raw_data {
                let decompressed = decompress_bitmap(data, bitmap_info, cast_lib, 0);
                match decompressed {
                    Ok(new_bitmap) => bitmap_manager.add_bitmap(new_bitmap),
                    Err(e) => {
                        warn!("[BMP] Failed to decode Raw data for member {}:{}: {}", cast_lib, number, e);
                        bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, 8, 0, PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))
                    }
                }
            } else {
                warn!("No bitmap chunk found for member {}", number);
                bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, 8, 0, PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))
            }
        }
    }

    fn extract_text_from_xmedia(data: &[u8]) -> Option<String> {
        let mut i = 0;

        while i < data.len() {
            // find '2C' which marks the start of text
            if data[i] != 0x2C {
                i += 1;
                continue;
            }

            let start = i + 1;

            // find following 03 byte
            let mut end = start;
            while end < data.len() && data[end] != 0x03 {
                end += 1;
            }

            // if 03 not found â†’ no valid text block
            if end >= data.len() {
                return None;
            }

            // extract text bytes
            let raw = &data[start..end];
            let mut text = String::new();

            for &b in raw {
                match b {
                    0x20..=0x7E => text.push(b as char), // printable ASCII
                    0x09 => text.push('\t'),             // preserve TAB
                    0x0D => text.push('\r'),             // preserve CR
                    0x0A => text.push('\n'),             // preserve LF
                    _ => {}                              // skip weird bytes
                }
            }

            let cleaned = text.trim().to_string();
            if !cleaned.is_empty() {
                return Some(cleaned);
            }

            i = end + 1;
        }

        None
    }

    fn scan_font_name_from_xmedia(xmedia: &XMediaChunk) -> Option<String> {
        let data = &xmedia.raw_data;

        for i in 0..data.len().saturating_sub(20) {
            // Look for the exact prefix
            if data[i..].starts_with(b"FFF Reaction") {
                // Extract until the null terminator
                let mut name = Vec::new();

                for &b in &data[i..] {
                    if b == 0 { break; }
                    if b.is_ascii_graphic() || b == b' ' {
                        name.push(b);
                    }
                }

                if !name.is_empty() {
                    return Some(String::from_utf8_lossy(&name).to_string());
                }
            }
        }

        None
    }

    fn extract_pfr(member_def: &CastMemberDef) -> Option<PfrFont> {
        member_def.children.iter()
            .find_map(|c| match c {
                Some(Chunk::XMedia(x)) if x.is_pfr_font() => x.parse_pfr_font(),
                _ => None
            })
    }

    fn resolve_font_name(chunk: &CastMemberChunk, pfr: &Option<PfrFont>, number: u32) -> String {
        if let Some(name) = chunk.member_info.as_ref().map(|i| i.name.clone()).filter(|n| !n.is_empty()) {
            return name;
        }

        if let Some(ref pfr) = pfr {
            if !pfr.font_name.is_empty() {
                return pfr.font_name.clone();
            }
        }

        if let Some(info) = chunk.specific_data.font_info() {
            if !info.name.is_empty() {
                return info.name.clone();
            }
        }

        format!("Font_{}", number)
    }

    fn render_pfr_to_bitmap(
        pfr: &PfrFont,
        bitmap_manager: &mut BitmapManager,
        target_height: usize,
    ) -> PfrBitmap {
        use crate::director::chunks::pfr1::{rasterizer, parse_pfr1_font_with_target};

        // Parse at target=0 to keep coordinates in ORU space. The rasterizer
        // handles ORU→pixel scaling using target_height / outline_res.
        let parsed_for_size = match parse_pfr1_font_with_target(&pfr.raw_data, 0) {
            Ok(p) => p,
            Err(_) => pfr.parsed.clone(),
        };

        // Use the PFR1 rasterizer to render the parsed font
        let rasterized = rasterizer::rasterize_pfr1_font(&parsed_for_size, target_height, 0);

        let bitmap_width = rasterized.bitmap_width as u16;
        let bitmap_height = rasterized.bitmap_height as u16;

        debug!(
            "🎨 Creating bitmap for PFR font '{}' ({}x{}, grid {}x{}, cell {}x{})",
            pfr.font_name,
            bitmap_width,
            bitmap_height,
            rasterized.grid_columns,
            rasterized.grid_rows,
            rasterized.cell_width,
            rasterized.cell_height,
        );

        // Create a 32-bit bitmap from the rasterized RGBA data
        let mut bitmap = Bitmap::new(
            bitmap_width,
            bitmap_height,
            32,
            32,
            0,
            PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
        );

        // Copy RGBA data
        let data_len = rasterized.bitmap_data.len().min(bitmap.data.len());
        bitmap.data[..data_len].copy_from_slice(&rasterized.bitmap_data[..data_len]);

        // Ensure transparent background is white (avoids black-square artifacts in text rendering)
        for i in (0..data_len).step_by(4) {
            let a = bitmap.data[i + 3];
            if a == 0 {
                bitmap.data[i] = 255;
                bitmap.data[i + 1] = 255;
                bitmap.data[i + 2] = 255;
            }
        }

        bitmap.use_alpha = true;

        debug!("✅ Finished assembling PFR bitmap ({} glyphs rendered).",
            parsed_for_size.glyphs.len() + parsed_for_size.bitmap_glyphs.len());

        let bitmap_ref = bitmap_manager.add_bitmap(bitmap);

        PfrBitmap {
            bitmap_ref,
            char_width: rasterized.cell_width as u16,
            char_height: rasterized.cell_height as u16,
            grid_columns: rasterized.grid_columns as u8,
            grid_rows: rasterized.grid_rows as u8,
            char_widths: Some(rasterized.char_widths),
            first_char: rasterized.first_char,
        }
    }

    fn log_ole_start(number: u32, cast_lib: u32, chunk: &CastMemberChunk) {
        debug!(
            "Processing Ole member #{} in cast lib {} (name: {})",
            number,
            cast_lib,
            chunk.member_info.as_ref().map(|x| x.name.as_str()).unwrap_or("")
        );
    }

    fn log_found_swf(number: u32, sig: &[u8], len: usize) {
        debug!("✅ Found SWF data in Ole member #{} (signature: {:?}, {} bytes)", number, sig, len);
    }

    fn log_found_swf_at_offset(number: u32, sig: &[u8]) {
        debug!("✅ Found SWF signature at offset 12 in Ole member #{}: {:?}", number, sig);
    }

    fn log_unknown_ole(number: u32, chunk: &CastMemberChunk) {
        let name = chunk.member_info.as_ref().map(|x| x.name.as_str()).unwrap_or("");
        web_sys::console::log_1(&format!(
            "Cast member #{} has unimplemented type: Ole (name: {})",
            number, name
        ).into());
    }

    /// Parse SWF stage dimensions from uncompressed SWF header.
    /// Returns (width, height) in pixels, or None if parsing fails.
    fn parse_swf_dimensions(data: &[u8]) -> Option<(u16, u16)> {
        if data.len() < 9 {
            return None;
        }
        // For compressed SWF (CWS/ZWS), we can't easily read the rect without decompressing
        let sig = &data[0..3];
        if sig != b"FWS" {
            return None; // Only uncompressed SWF for now
        }
        // SWF RECT starts at byte 8
        // RECT format: Nbits (5 bits) then 4 fields of Nbits each
        let rect_start = 8;
        if data.len() <= rect_start {
            return None;
        }
        let nbits = (data[rect_start] >> 3) as usize;
        let total_bits = 5 + nbits * 4;
        let total_bytes = (total_bits + 7) / 8;
        if data.len() < rect_start + total_bytes {
            return None;
        }

        // Read bit fields
        let mut bit_pos = rect_start * 8 + 5; // skip 5-bit nbits field
        let read_bits = |pos: usize, n: usize| -> i32 {
            let mut val: i32 = 0;
            for i in 0..n {
                let byte_idx = (pos + i) / 8;
                let bit_idx = 7 - ((pos + i) % 8);
                if (data[byte_idx] >> bit_idx) & 1 != 0 {
                    val |= 1 << (n - 1 - i);
                }
            }
            // Sign extend
            if n > 0 && (val >> (n - 1)) & 1 != 0 {
                val |= !0 << n;
            }
            val
        };

        let x_min = read_bits(bit_pos, nbits); bit_pos += nbits;
        let x_max = read_bits(bit_pos, nbits); bit_pos += nbits;
        let y_min = read_bits(bit_pos, nbits); bit_pos += nbits;
        let y_max = read_bits(bit_pos, nbits);
        let _ = bit_pos;

        // SWF uses twips (1/20 pixel)
        let width = ((x_max - x_min) / 20) as u16;
        let height = ((y_max - y_min) / 20) as u16;
        Some((width, height))
    }

    fn make_swf_member(number: u32, chunk: &CastMemberChunk, data: Vec<u8>) -> CastMember {
        let flash_info = chunk.specific_data.flash_info().cloned();
        let reg_point = if let Some(ref info) = flash_info {
            if info.center_reg_point {
                (info.reg_point.0 as i16, info.reg_point.1 as i16)
            } else {
                (info.origin_h as i16, info.origin_v as i16)
            }
        } else {
            // OLE-wrapped SWF: default to center registration from SWF dimensions
            if let Some((w, h)) = Self::parse_swf_dimensions(&data) {
                ((w / 2) as i16, (h / 2) as i16)
            } else {
                (0, 0)
            }
        };

        CastMember {
            number,
            name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
            comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
            member_type: CastMemberType::Flash(FlashMember { data, reg_point, flash_info }),
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }

    fn get_first_child_bytes(member_def: &CastMemberDef) -> Option<Vec<u8>> {
        if let Some(Some(ch)) = member_def.children.get(0) {
            if let Some(bytes) = ch.as_bytes() {
                return Some(bytes.to_vec());
            }
        }
        None
    }

    /// Create an empty W3D scene with a DefaultView camera, matching Director's behavior.
    pub(crate) fn create_empty_w3d_scene() -> crate::director::chunks::w3d::types::W3dScene {
        use crate::director::chunks::w3d::types::*;
        let mut scene = W3dScene::default();
        // Director always has a DefaultShader that cannot be deleted
        scene.shaders.push(W3dShader {
            name: "DefaultShader".to_string(),
            ..Default::default()
        });
        // Director always creates a DefaultView camera in empty 3D members
        scene.nodes.push(W3dNode {
            name: "DefaultView".to_string(),
            node_type: W3dNodeType::View,
            parent_name: "World".to_string(),
            resource_name: String::new(),
            model_resource_name: String::new(),
            shader_name: String::new(),
            near_plane: 1.0,
            far_plane: 10000.0,
            fov: 30.0,
            screen_width: 640,
            screen_height: 480,
            transform: [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,100.0,1.0],
        });
        // Default ambient light
        scene.lights.push(W3dLight {
            name: "DefaultAmbient".to_string(),
            light_type: W3dLightType::Ambient,
            color: [0.3, 0.3, 0.3],
            enabled: true,
            spot_angle: 90.0,
            attenuation: [1.0, 0.0, 0.0],
        });
        // Default directional light (IFX default: 0.75)
        scene.lights.push(W3dLight {
            name: "DefaultDirectional".to_string(),
            light_type: W3dLightType::Directional,
            color: [0.75, 0.75, 0.75],
            enabled: true,
            spot_angle: 90.0,
            attenuation: [1.0, 0.0, 0.0],
        });
        // Light node for the directional light — rotated to point from upper-right
        scene.nodes.push(W3dNode {
            name: "DefaultDirectional".to_string(),
            node_type: W3dNodeType::Light,
            parent_name: "World".to_string(),
            resource_name: "DefaultDirectional".to_string(),
            model_resource_name: String::new(),
            shader_name: String::new(),
            near_plane: 1.0, far_plane: 10000.0, fov: 30.0,
            screen_width: 640, screen_height: 480,
            // Rotation: Z-axis points toward (0.5, 1.0, 0.7) normalized
            // -Z axis = (-0.37, -0.74, -0.52) is the light direction
            transform: [
                0.88, 0.0, -0.47, 0.0,
                -0.35, 0.67, -0.65, 0.0,
                0.32, 0.74, 0.59, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ],
        });
        scene
    }

    /// Check if OLE specific_data_raw is a Shockwave3D member.
    /// Format: 4-byte BE string length + "shockwave3d" + 3DPR data
    fn is_shockwave3d_ole(raw: &[u8]) -> bool {
        if raw.len() < 15 { return false; }
        let str_len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if str_len == 0 || raw.len() < 4 + str_len { return false; }
        std::str::from_utf8(&raw[4..4 + str_len]).ok() == Some("shockwave3d")
    }

    /// Check if OLE specific_data_raw is a SWA (Shockwave Audio) member.
    /// Format: 4-byte BE string length + "swa" + Xtra-specific data with file path
    fn is_swa_ole(raw: &[u8]) -> bool {
        if raw.len() < 7 { return false; }
        let str_len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if str_len == 0 || raw.len() < 4 + str_len { return false; }
        std::str::from_utf8(&raw[4..4 + str_len]).ok() == Some("swa")
    }

    /// Try to parse OLE specific_data_raw as a vectorShape member.
    /// Format: 4-byte BE string length + "vectorShape" + 4-byte FLSH size + "FLSH" fourCC + 4-byte size + payload
    fn try_parse_vector_shape(raw: &[u8], number: u32, chunk: &CastMemberChunk) -> Option<CastMember> {
        if raw.len() < 15 {
            return None;
        }
        // Read length-prefixed type string
        let str_len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if str_len == 0 || raw.len() < 4 + str_len {
            return None;
        }
        let type_str = std::str::from_utf8(&raw[4..4 + str_len]).ok()?;
        if type_str != "vectorShape" {
            return None;
        }

        // Parse FLSH block: after type string, we have 4-byte size + "FLSH" fourCC + payload
        let flsh_start = 4 + str_len;
        if raw.len() < flsh_start + 8 {
            debug!("OLE member #{}: vectorShape too short for FLSH header", number);
            return None;
        }
        let flsh_fourcc = &raw[flsh_start + 4..flsh_start + 8];
        if flsh_fourcc != b"FLSH" {
            debug!("OLE member #{}: vectorShape missing FLSH fourCC", number);
            return None;
        }

        // FLSH payload starts after: 4-byte size + "FLSH" = 8 bytes
        // (the size field covers everything from "FLSH" onwards)
        let payload = &raw[flsh_start + 8..];
        let vector_member = Self::parse_flsh_payload(payload);

        web_sys::console::log_1(
            &format!(
                "OLE member #{} identified as vectorShape: {} vertices, strokeWidth={}, fillMode={}, closed={}, bbox=({},{},{},{})",
                number,
                vector_member.vertices.len(),
                vector_member.stroke_width,
                vector_member.fill_mode,
                vector_member.closed,
                vector_member.bbox_left, vector_member.bbox_top,
                vector_member.bbox_right, vector_member.bbox_bottom,
            ).into(),
        );

        Some(CastMember {
            number,
            name: chunk
                .member_info
                .as_ref()
                .map(|x| x.name.to_owned())
                .unwrap_or_default(),
            comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
            member_type: CastMemberType::VectorShape(vector_member),
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        })
    }

    /// Parse the FLSH payload into VectorShapeMember.
    /// Fixed header (160 bytes) + 4 colors (64 bytes) + vertex list.
    fn parse_flsh_payload(data: &[u8]) -> VectorShapeMember {
        let read_u32 = |off: usize| -> u32 {
            if off + 4 <= data.len() {
                u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
            } else {
                0
            }
        };
        let read_f32 = |off: usize| -> f32 {
            if off + 4 <= data.len() {
                f32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]])
            } else {
                0.0
            }
        };

        // Fixed header fields
        let num_vertices = read_u32(100) as usize;
        let closed = read_u32(112) != 0;
        let fill_mode = read_u32(124);
        let stroke_width = read_f32(128);

        // 4 colors starting at offset 160, each: 4-byte marker (0x12) + 3x 4-byte RGB
        let parse_color = |base: usize| -> (u8, u8, u8) {
            let r = read_u32(base + 4) as u8;
            let g = read_u32(base + 8) as u8;
            let b = read_u32(base + 12) as u8;
            (r, g, b)
        };
        let stroke_color = parse_color(160);
        let fill_color = parse_color(176);
        let bg_color = parse_color(192);
        let end_color = parse_color(208);

        // Vertex list starts at offset 224
        // Format: 4-byte list type (0x07) + 4-byte numVertices
        // Then per-vertex entries, each prefixed by: entry_type(4) + item_count(4)
        let mut vertices = Vec::with_capacity(num_vertices);
        let mut pos = 224 + 8; // skip list marker + count

        for i in 0..num_vertices {
            if pos + 8 > data.len() {
                break;
            }

            // ALL vertices have an entry header: type(4) + item_count(4)
            // (0x0A = property list, 0x03 = 3 items: vertex, handle1, handle2)
            pos += 8;

            if i == 0 {
                // First vertex uses string keys: type(4) + strlen(4) + string + datasize(4) + data(8)
                let vertex = Self::parse_string_keyed_point(data, &mut pos);
                let handle1 = Self::parse_string_keyed_point(data, &mut pos);
                let handle2 = Self::parse_string_keyed_point(data, &mut pos);

                vertices.push(VectorShapeVertex {
                    x: vertex.0 as f32,
                    y: vertex.1 as f32,
                    handle1_x: handle1.0 as f32,
                    handle1_y: handle1.1 as f32,
                    handle2_x: handle2.0 as f32,
                    handle2_y: handle2.1 as f32,
                });
            } else {
                // Subsequent vertices use hash keys: type(4) + hash(4) + datasize(4) + data(8)
                let vertex = Self::parse_hash_keyed_point(data, &mut pos);
                let handle1 = Self::parse_hash_keyed_point(data, &mut pos);
                let handle2 = Self::parse_hash_keyed_point(data, &mut pos);

                vertices.push(VectorShapeVertex {
                    x: vertex.0 as f32,
                    y: vertex.1 as f32,
                    handle1_x: handle1.0 as f32,
                    handle1_y: handle1.1 as f32,
                    handle2_x: handle2.0 as f32,
                    handle2_y: handle2.1 as f32,
                });
            }
        }

        // Compute bounding box from vertices + control points + stroke padding
        let mut bbox_left = f32::MAX;
        let mut bbox_top = f32::MAX;
        let mut bbox_right = f32::MIN;
        let mut bbox_bottom = f32::MIN;
        for v in &vertices {
            // Include vertex position
            bbox_left = bbox_left.min(v.x);
            bbox_top = bbox_top.min(v.y);
            bbox_right = bbox_right.max(v.x);
            bbox_bottom = bbox_bottom.max(v.y);
            // Include absolute control points (vertex + handle offsets)
            for &(cx, cy) in &[
                (v.x + v.handle1_x, v.y + v.handle1_y),
                (v.x + v.handle2_x, v.y + v.handle2_y),
            ] {
                bbox_left = bbox_left.min(cx);
                bbox_top = bbox_top.min(cy);
                bbox_right = bbox_right.max(cx);
                bbox_bottom = bbox_bottom.max(cy);
            }
        }
        // Add stroke padding
        let pad = stroke_width / 2.0;
        bbox_left -= pad;
        bbox_top -= pad;
        bbox_right += pad;
        bbox_bottom += pad;

        // Fallback for empty vertex lists
        if vertices.is_empty() {
            bbox_left = 0.0;
            bbox_top = 0.0;
            bbox_right = 0.0;
            bbox_bottom = 0.0;
        }

        web_sys::console::log_1(
            &format!(
                "  FLSH parsed: stroke=({},{},{}), strokeW={}, fillMode={}, closed={}, verts={}",
                stroke_color.0, stroke_color.1, stroke_color.2,
                stroke_width, fill_mode, closed, vertices.len(),
            ).into(),
        );
        for (i, v) in vertices.iter().enumerate() {
            web_sys::console::log_1(
                &format!(
                    "  vertex[{}]: ({}, {}) h1=({}, {}) h2=({}, {})",
                    i, v.x, v.y, v.handle1_x, v.handle1_y, v.handle2_x, v.handle2_y,
                ).into(),
            );
        }

        VectorShapeMember {
            stroke_color,
            fill_color,
            bg_color,
            end_color,
            stroke_width,
            fill_mode,
            closed,
            vertices,
            bbox_left,
            bbox_top,
            bbox_right,
            bbox_bottom,
        }
    }

    /// Parse a string-keyed point entry from FLSH vertex data.
    /// Format: type(4) + strlen(4) + string_bytes + datasize(4) + locV(4) + locH(4)
    /// FLSH stores points as (locV, locH) i.e. (y, x). We return (x, y).
    fn parse_string_keyed_point(data: &[u8], pos: &mut usize) -> (i32, i32) {
        // type marker (4 bytes, value 0x02)
        *pos += 4;
        // string length
        let str_len = if *pos + 4 <= data.len() {
            u32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]]) as usize
        } else {
            0
        };
        *pos += 4;
        // skip string bytes
        *pos += str_len;
        // data size (4 bytes, should be 8)
        *pos += 4;
        // FLSH stores locV first, then locH
        let loc_v = if *pos + 4 <= data.len() {
            i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]])
        } else {
            0
        };
        *pos += 4;
        let loc_h = if *pos + 4 <= data.len() {
            i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]])
        } else {
            0
        };
        *pos += 4;
        (loc_h, loc_v) // return as (x, y)
    }

    /// Parse a hash-keyed point entry from FLSH vertex data.
    /// Format: type(4) + hash(4) + datasize(4) + locV(4) + locH(4)
    /// FLSH stores points as (locV, locH) i.e. (y, x). We return (x, y).
    fn parse_hash_keyed_point(data: &[u8], pos: &mut usize) -> (i32, i32) {
        // type marker (4 bytes, value 0x02)
        *pos += 4;
        // hash key (4 bytes, e.g. 0x80000000 for vertex)
        *pos += 4;
        // data size (4 bytes, should be 8)
        *pos += 4;
        // FLSH stores locV first, then locH
        let loc_v = if *pos + 4 <= data.len() {
            i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]])
        } else {
            0
        };
        *pos += 4;
        let loc_h = if *pos + 4 <= data.len() {
            i32::from_be_bytes([data[*pos], data[*pos + 1], data[*pos + 2], data[*pos + 3]])
        } else {
            0
        };
        *pos += 4;
        (loc_h, loc_v) // return as (x, y)
    }

    fn try_parse_swf(bytes: Vec<u8>, number: u32, chunk: &CastMemberChunk) -> Option<CastMember> {
        if bytes.len() < 3 {
            return None;
        }

        let sig = &bytes[0..3];

        let is_swf = sig == b"FWS" || sig == b"CWS" || sig == b"ZWS";
        if is_swf {
            Self::log_found_swf(number, sig, bytes.len());
            return Some(Self::make_swf_member(number, chunk, bytes));
        }

        // Try offset 12 SWF (OLE wrapped SWF)
        if bytes.len() > 15 {
            let sig2 = &bytes[12..15];
            let is_swf2 = sig2 == b"FWS" || sig2 == b"CWS" || sig2 == b"ZWS";
            if is_swf2 {
                Self::log_found_swf_at_offset(number, sig2);
                return Some(Self::make_swf_member(number, chunk, bytes[12..].to_vec()));
            }
        }

        None
    }

    fn scan_children_for_ole(
        member_def: &CastMemberDef,
        number: u32,
        chunk: &CastMemberChunk,
        bitmap_manager: &mut BitmapManager,
    ) -> Option<CastMember>
    {
        for opt_child in &member_def.children {
            let Some(Chunk::XMedia(xm)) = opt_child else { continue };

            let member_name = chunk.member_info.as_ref().map(|i| i.name.as_str()).unwrap_or("");
            web_sys::console::log_1(&format!("Checking XMedia child (member #{}, name='{}', {} bytes)", number, member_name, xm.raw_data.len()).into());

            // 1) If SWF: return SWF
            if let Some(cm) = Self::try_parse_swf(xm.raw_data.to_vec(), number, chunk) {
                web_sys::console::log_1(&"Detected as SWF".into());
                return Some(cm);
            }

            // 2) Check if styled text (XMED format)
            // Only parse as styled text if the Ole type string is "text" or empty
            // (avoid mis-parsing raw binary data like lightmap coordinates as text)
            let ole_type = if chunk.specific_data_raw.len() >= 4 {
                let str_len = u32::from_be_bytes([chunk.specific_data_raw[0], chunk.specific_data_raw[1], chunk.specific_data_raw[2], chunk.specific_data_raw[3]]) as usize;
                if str_len > 0 && chunk.specific_data_raw.len() >= 4 + str_len {
                    std::str::from_utf8(&chunk.specific_data_raw[4..4 + str_len]).unwrap_or("")
                } else {
                    ""
                }
            } else {
                ""
            };
            let is_text_ole = ole_type.is_empty() || ole_type == "text";
            if is_text_ole {
                if let Some(styled_text) = xm.parse_styled_text() {
                    web_sys::console::log_1(&"Detected as XMED styled text".into());
                    return Some(Self::create_text_member_from_xmed(
                        number,
                        chunk,
                        styled_text,
                    ));
                }
            }

            // 3) Shockwave3D — IFX IFF container; not a font
            if xm.is_shockwave3d() {
                web_sys::console::log_1(&format!(
                    "Detected as Shockwave3D (IFX) member #{}, {} bytes",
                    number, xm.raw_data.len()
                ).into());
                let w3d_data = xm.raw_data.clone();
                let parsed_scene = if !w3d_data.is_empty() {
                    match crate::director::chunks::w3d::parse_w3d(&w3d_data) {
                        Ok(mut scene) => {
                            web_sys::console::log_1(&format!("W3D parsed: {} materials, {} nodes, {} meshes",
                                scene.materials.len(), scene.nodes.len(), scene.clod_meshes.len()).into());
                            // Ensure DefaultShader exists
                            if !scene.shaders.iter().any(|s| s.name == "DefaultShader") {
                                scene.shaders.push(crate::director::chunks::w3d::types::W3dShader {
                                    name: "DefaultShader".to_string(),
                                    ..Default::default()
                                });
                            }
                            Some(std::rc::Rc::new(scene))
                        }
                        Err(e) => {
                            web_sys::console::log_1(&format!("W3D parse error: {}", e).into());
                            Some(std::rc::Rc::new(Self::create_empty_w3d_scene()))
                        }
                    }
                } else {
                    Some(std::rc::Rc::new(Self::create_empty_w3d_scene()))
                };
                let info = Shockwave3dInfo {
                    loops: false, duration: 0, direct_to_stage: false,
                    animation_enabled: false, preload: false,
                    reg_point: (0, 0), default_rect: (0, 0, 0, 0),
                    camera_position: None, camera_rotation: None,
                    bg_color: None, ambient_color: None,
                };
                let source_scene = parsed_scene.clone();
                return Some(CastMember {
                    number,
                    name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                    comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                    member_type: { let rs = Shockwave3dRuntimeState::from_info(&info, parsed_scene.as_deref()); CastMemberType::Shockwave3d(Shockwave3dMember { info, w3d_data, source_scene, parsed_scene, runtime_state: rs, converted_from_text: false, text3d_state: None, text3d_source: None }) },
                    color: ColorRef::PaletteIndex(255),
                    bg_color: ColorRef::PaletteIndex(0),
                });
            }

            // 4) Check if this is a real PFR font or just text content
            let has_pfr = Self::extract_pfr(member_def).is_some();
            if has_pfr {
                web_sys::console::log_1(&"Detected as PFR font".into());
                return Some(Self::parse_xmedia_font(member_def, number, chunk, xm, bitmap_manager));
            }

            // 5) No PFR font data — create a TextMember from the XMedia text content
            // Use the proper XMED parser to get clean text, fall back to empty
            let text = xm.parse_styled_text()
                .map(|st| st.text.clone())
                .unwrap_or_default();
            let member_name = chunk.member_info.as_ref()
                .map(|x| x.name.to_owned())
                .unwrap_or_default();
            let mut text_member = TextMember::new();
            text_member.text = text;
            return Some(CastMember {
                number,
                name: member_name,
                comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                member_type: CastMemberType::Text(text_member),
                color: ColorRef::PaletteIndex(255),
                bg_color: ColorRef::PaletteIndex(0),
            });
        }
        None
    }

    fn parse_xmedia_font(
        member_def: &CastMemberDef,
        number: u32,
        chunk: &CastMemberChunk,
        xm: &XMediaChunk,
        bitmap_manager: &mut BitmapManager,
    ) -> CastMember {
        let pfr = Self::extract_pfr(member_def);

        // Extract preview text using proper XMED parser.
        // PFR fonts don't have preview text.
        let preview_text = if pfr.is_some() {
            String::new()
        } else {
            xm.parse_styled_text()
                .map(|st| st.text.clone())
                .filter(|s| s.len() > 3)
                .unwrap_or_default()
        };
        let preview_font_name = Self::scan_font_name_from_xmedia(xm);
        let font_name = Self::resolve_font_name(chunk, &pfr, number);

        let info_and_bitmap = Self::build_font_info_and_bitmap(
            pfr,
            chunk,
            &font_name,
            bitmap_manager,
        );

        let (font_info, bitmap_ref, char_w, char_h, gc, gr, char_widths, first_char, pfr_parsed, pfr_data) = info_and_bitmap;

        let member_name = chunk
            .member_info
            .as_ref()
            .map(|x| x.name.to_owned())
            .unwrap_or_default();

        debug!(
            "FontMember #{} name='{}' font_name='{}' preview_text='{}' preview_font_name={:?} \
             fixed_line_space=14 top_spacing=0 char_width={:?} char_height={:?} \
             grid_columns={:?} grid_rows={:?} first_char_num={:?} char_widths_len={} \
             bitmap_ref={:?} pfr_parsed={} pfr_data_len={}",
            number, member_name, font_name, preview_text, preview_font_name,
            char_w, char_h, gc, gr, first_char,
            char_widths.as_ref().map_or(0, |v| v.len()),
            bitmap_ref, pfr_parsed.is_some(), pfr_data.as_ref().map_or(0, |d| d.len()),
        );

        CastMember {
            number,
            name: member_name,
            comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
            member_type: CastMemberType::Font(FontMember {
                font_info,
                preview_text,
                preview_font_name,
                preview_html_spans: Vec::new(),
                fixed_line_space: 14,
                top_spacing: 0,
                bitmap_ref,
                char_width: char_w,
                char_height: char_h,
                grid_columns: gc,
                grid_rows: gr,
                char_widths,
                first_char_num: first_char,
                alignment: TextAlignment::Left,
                pfr_parsed,
                pfr_data,
            }),
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }

    fn create_text_member_from_xmed(
        number: u32,
        chunk: &CastMemberChunk,
        styled_text: crate::director::chunks::xmedia::XmedStyledText,
    ) -> CastMember {
        use crate::player::handlers::datum_handlers::cast_member::font::TextAlignment;

        debug!(
            "[XMED] Creating TextMember from XMED styled text (member #{})", number
        );

        let alignment_str = match styled_text.alignment {
            TextAlignment::Left => "left",
            TextAlignment::Center => "center",
            TextAlignment::Right => "right",
            TextAlignment::Justify => "justify",
        };

        // Use first span font face, but member fontSize should track the largest styled size.
        let (font_name, font_size) = if !styled_text.styled_spans.is_empty() {
            let first_style = &styled_text.styled_spans[0].style;
            let max_span_size = styled_text
                .styled_spans
                .iter()
                .filter_map(|s| s.style.font_size)
                .filter(|s| *s > 0)
                .max()
                .unwrap_or(12);
            (
                first_style.font_face.clone().unwrap_or_else(|| "Arial".to_string()),
                max_span_size as u16,
            )
        } else {
            ("Arial".to_string(), 12)
        };

        debug!(
            "[XMED]   text='{}', alignment={}, font='{}', size={}, spans={}, word_wrap={}",
            styled_text.text, alignment_str, font_name, font_size, styled_text.styled_spans.len(),
            styled_text.word_wrap
        );

        // Get TextInfo from specific_data if available; otherwise synthesize a default one
        // so runtime properties like centerRegPoint are always present on parsed text members.
        let text_info_from_chunk = chunk.specific_data.text_info().cloned();
        let raw_looks_like_text_info = TextInfo::looks_like_text_info(chunk.specific_data_raw.as_slice());
        let text_info_from_raw = if text_info_from_chunk.is_none() && raw_looks_like_text_info {
            Some(TextInfo::from(chunk.specific_data_raw.as_slice()))
        } else {
            None
        };
        let text_info_from_chunk = text_info_from_chunk.or(text_info_from_raw);
        let field_info_from_chunk = chunk.specific_data.field_info();
        let mut text_info = text_info_from_chunk.unwrap_or_else(|| {
            let mut info = TextInfo::default();
            if let Some(field_info) = field_info_from_chunk {
                info.box_type = field_info.box_type as u32;
                info.scroll_top = field_info.scroll as u32;
                info.auto_tab = field_info.auto_tab();
                info.editable = field_info.editable();
                info.dont_wrap = !field_info.wordwrap();
                info.width = field_info.width() as u32;
                info.height = field_info.height() as u32;
            }
            info
        });
        let mut box_w = if text_info.width > 0 { text_info.width as u16 } else { 0 };
        let mut box_h = if text_info.height > 0 { text_info.height as u16 } else { 0 };

        // Fallback for older text member formats: parse raw text member data for dimensions.
        if box_w == 0 || box_h == 0 {
            if let Some(text_member_data) = TextMemberData::from_raw_bytes(chunk.specific_data_raw.as_slice()) {
                if box_w == 0 && text_member_data.width > 0 {
                    box_w = text_member_data.width as u16;
                }
                if box_h == 0 && text_member_data.height > 0 {
                    box_h = text_member_data.height as u16;
                }
            }
        }

        if box_w == 0 { box_w = 100; }
        if box_h == 0 { box_h = 20; }

        // Keep synthesized TextInfo dimensions aligned with effective member box.
        text_info.width = box_w as u32;
        text_info.height = box_h as u32;

        let box_type = text_info.box_type_str().trim_start_matches('#').to_string();
        let word_wrap = text_info.word_wrap();
        let xmed_bg_color = styled_text.bg_color;
        let text_member = TextMember {
            text: styled_text.text.clone(),
            html_source: String::new(),
            rtf_source: String::new(),
            alignment: alignment_str.to_string(),
            box_type,
            word_wrap,
            anti_alias: true,
            font: font_name,
            font_style: Vec::new(),
            font_size,
            fixed_line_space: if styled_text.line_spacing > 0 {
                styled_text.line_spacing as u16
            } else {
                styled_text.fixed_line_space
            },
            top_spacing: styled_text.top_spacing as i16,
            bottom_spacing: styled_text.bottom_spacing as i16,
            width: box_w,
            height: box_h,
            char_spacing: styled_text.styled_spans.first()
                .map(|s| s.style.char_spacing as i32)
                .unwrap_or(0),
            tab_stops: Vec::new(),
            html_styled_spans: styled_text.styled_spans,
            info: Some(text_info),
            w3d: None,
        };

        let member_name = chunk
            .member_info
            .as_ref()
            .map(|x| x.name.to_owned())
            .unwrap_or_default();

        debug!(
            "[XMED] TextMember #{} name='{}' text='{}' alignment='{}' box_type='{}' word_wrap={} \
             anti_alias={} font='{}' font_style={:?} font_size={} fixed_line_space={} \
             top_spacing={} width={} height={} styled_spans={}",
            number,
            member_name,
            text_member.text,
            text_member.alignment,
            text_member.box_type,
            text_member.word_wrap,
            text_member.anti_alias,
            text_member.font,
            text_member.font_style,
            text_member.font_size,
            text_member.fixed_line_space,
            text_member.top_spacing,
            text_member.width,
            text_member.height,
            text_member.html_styled_spans.len(),
        );

        // Preserve XMED foreColor at the member level so it persists
        // even when Lingo sets member.text or member.html (which may clear styled span colors)
        let member_color = text_member.html_styled_spans.first()
            .and_then(|s| s.style.color)
            .map(|c| ColorRef::Rgb(
                ((c >> 16) & 0xFF) as u8,
                ((c >> 8) & 0xFF) as u8,
                (c & 0xFF) as u8,
            ))
            .unwrap_or(ColorRef::PaletteIndex(255));

        // Extract XMED backColor from Section 0x0000 document header (indices 30-32).
        let member_bg_color = xmed_bg_color
            .map(|(r, g, b)| ColorRef::Rgb(r, g, b))
            .unwrap_or(ColorRef::PaletteIndex(0));

        CastMember {
            number,
            name: member_name,
            comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
            member_type: CastMemberType::Text(text_member),
            color: member_color,
            bg_color: member_bg_color,
        }
    }

    fn build_font_info_and_bitmap(
        pfr: Option<PfrFont>,
        chunk: &CastMemberChunk,
        default_name: &str,
        bitmap_manager: &mut BitmapManager,
    ) -> (
        FontInfo,
        Option<BitmapRef>,
        Option<u16>, // char width
        Option<u16>, // char height
        Option<u8>, // grid columns
        Option<u8>, // grid rows
        Option<Vec<u16>>, // char widths
        Option<u8>, // first_char_num
        Option<crate::director::chunks::pfr1::types::Pfr1ParsedFont>,
        Option<Vec<u8>>,
        ) {
        let specific_bytes = chunk.specific_data_raw.clone();

        if let Some(pfr) = pfr {
            let requested_size = chunk
                .specific_data
                .font_info()
                .map(|fi| fi.size)
                .unwrap_or(0);
            let target_height = if requested_size > 0 {
                requested_size as usize
            } else {
                16usize
            };

            debug!(
                "[font.load] font='{}' FontInfo.size={} target_height={}",
                if pfr.font_name.is_empty() { default_name } else { &pfr.font_name },
                requested_size, target_height
            );

            let bmp = Self::render_pfr_to_bitmap(&pfr, bitmap_manager, target_height);

            let info = FontInfo {
                font_id: 0,
                size: target_height as u16,
                style: 0,
                name: if pfr.font_name.is_empty() {
                    default_name.to_string()
                } else {
                    pfr.font_name.clone()
                }
            };

            debug!("Rendered PFR: {:?}", info);

            return (
                info,
                Some(bmp.bitmap_ref),
                Some(bmp.char_width),
                Some(bmp.char_height),
                Some(bmp.grid_columns),
                Some(bmp.grid_rows),
                bmp.char_widths,
                Some(bmp.first_char),
                Some(pfr.parsed.clone()),
                Some(pfr.raw_data.clone()),
            );
        }

        if FontInfo::looks_like_real_font_data(&specific_bytes) {
            let info = chunk
                .specific_data
                .font_info()
                .map(|fi| fi.clone().with_default_name(default_name))
                .unwrap_or_else(|| FontInfo::minimal(default_name));

            return (info, None, None, None, None, None, None, None, None, None);
        }

        // fallback
        (FontInfo::minimal(default_name), None, None, None, None, None, None, None, None, None)
    }

    pub fn get_script_id(&self) -> Option<u32> {
        match &self.member_type {
            CastMemberType::Bitmap(bitmap) => {
                if bitmap.script_id > 0 {
                    Some(bitmap.script_id)
                } else {
                    None
                }
            }
            CastMemberType::Button(button) => {
                if button.script_id > 0 {
                    Some(button.script_id)
                } else {
                    None
                }
            }
            CastMemberType::Shape(shape) => {
                if shape.script_id > 0 {
                    Some(shape.script_id)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_member_script_ref(&self) -> Option<&CastMemberRef> {
        match &self.member_type {
            CastMemberType::Bitmap(bitmap) => bitmap.member_script_ref.as_ref(),
            CastMemberType::Button(button) => button.member_script_ref.as_ref(),
            CastMemberType::Shape(shape) => shape.member_script_ref.as_ref(),
            _ => None,
        }
    }

    pub fn set_member_script_ref(&mut self, script_ref: CastMemberRef) {
        match &mut self.member_type {
            CastMemberType::Bitmap(bitmap) => {
                bitmap.member_script_ref = Some(script_ref);
            }
            CastMemberType::Button(button) => {
                button.member_script_ref = Some(script_ref);
            }
            CastMemberType::Shape(shape) => {
                shape.member_script_ref = Some(script_ref);
            }
            _ => {}
        }
    }

    pub fn from(
        cast_lib: u32,
        number: u32,
        member_def: &CastMemberDef,
        lctx: &Option<ScriptContext>,
        bitmap_manager: &mut BitmapManager,
        dir_version: u16,
        palette_id_offset: i16,
        font_table: &HashMap<u16, String>,
    ) -> CastMember {
        let chunk = &member_def.chunk;

        let member_type = match chunk.member_type {
            MemberType::Text => {
                let text_chunk = member_def.children.iter()
                    .find_map(|c| c.as_ref().and_then(|ch| ch.as_text()))
                    .expect("No text chunk found for text member");
                let raw = chunk.specific_data_raw.as_slice();
                let field_info = FieldInfo::from(raw);
                debug!(
                    "[FIELD_INFO] rect=({},{},{},{}) text_height={} max_height={} border={} margin={} box_shadow={} box_type={} scroll={} flags=0x{:02X} raw_len={}",
                    field_info.rect_left, field_info.rect_top, field_info.rect_right, field_info.rect_bottom,
                    field_info.text_height,
                    field_info.max_height,
                    field_info.border,
                    field_info.margin,
                    field_info.box_drop_shadow,
                    field_info.box_type,
                    field_info.scroll,
                    field_info.flags,
                    raw.len(),
                );
                let mut field_member = FieldMember::from_field_info(&field_info);
                field_member.text = text_chunk.text.clone();

                // Parse STXT formatting data to extract actual fontId, fontSize, and style
                let formatting_runs = text_chunk.parse_formatting_runs();
                if let Some(first_run) = formatting_runs.first() {
                    // Extract foreground color from STXT formatting run
                    let fg_r = (first_run.color_r >> 8) as u8;
                    let fg_g = (first_run.color_g >> 8) as u8;
                    let fg_b = (first_run.color_b >> 8) as u8;
                    field_member.fore_color = Some(ColorRef::Rgb(fg_r, fg_g, fg_b));

                    field_member.font_id = Some(first_run.font_id);
                    // Resolve STXT font_id to font name via Fmap font table
                    if let Some(font_name) = font_table.get(&first_run.font_id) {
                        field_member.font = font_name.clone();
                    }
                    debug!(
                        "[field.font] STXT font_id={} -> Fmap='{}' (table has {} entries)",
                        first_run.font_id,
                        if field_member.font.is_empty() { "<NOT FOUND>" } else { &field_member.font },
                        font_table.len(),
                    );
                    if first_run.font_size > 0 {
                        field_member.font_size = first_run.font_size;
                    }
                    // Use STXT run's height as line spacing — this is Director's
                    // computed line height (ascent + descent + leading) for the run.
                    if first_run.height > 0 {
                        field_member.fixed_line_space = first_run.height;
                    }
                    if first_run.style != 0 {
                        let mut styles = Vec::new();
                        if (first_run.style & 0x01) != 0 {
                            styles.push("bold");
                        }
                        if (first_run.style & 0x02) != 0 {
                            styles.push("italic");
                        }
                        if (first_run.style & 0x04) != 0 {
                            styles.push("underline");
                        }
                        if styles.is_empty() {
                            field_member.font_style = "plain".to_string();
                        } else {
                            field_member.font_style = styles.join(" ");
                        }
                    }
                }

                debug!(
                    "FieldMember text='{}' alignment='{}' word_wrap={} font='{}' \
                     font_style='{}' font_size={} font_id={:?} fixed_line_space={} \
                     top_spacing={} box_type='{}' anti_alias={} width={} \
                     auto_tab={} editable={} border={} fore_color={:?} back_color={:?} formatting_runs={}",
                    field_member.text, field_member.alignment, field_member.word_wrap,
                    field_member.font, field_member.font_style, field_member.font_size,
                    field_member.font_id, field_member.fixed_line_space,
                    field_member.top_spacing, field_member.box_type, field_member.anti_alias,
                    field_member.width, field_member.auto_tab, field_member.editable,
                    field_member.border, field_member.fore_color, field_member.back_color,
                    formatting_runs.len(),
                );

                CastMemberType::Field(field_member)
            }
            MemberType::Script => {
                let member_info = chunk.member_info.as_ref().unwrap();
                let mut script_id = member_info.header.script_id;
                let script_type = chunk.specific_data.script_type().unwrap();
                let has_script = lctx.as_ref()
                    .map(|ctx| ctx.scripts.contains_key(&script_id))
                    .unwrap_or(false);

                // Note: script_id == 0 means the script was recycled/deleted.
                // Do NOT fall back to using the member number — the Lscr chunk at that
                // index may contain stale bytecode from before the script was recycled.

                let has_script = lctx.as_ref()
                    .map(|ctx| ctx.scripts.contains_key(&script_id))
                    .unwrap_or(false);

                if has_script {
                    CastMemberType::Script(ScriptMember {
                        script_id,
                        script_type,
                        name: member_info.name.clone(),
                    })
                } else {
                    web_sys::console::warn_1(&format!("Script member {}: script_id {} not found in Lctx, skipping", number, script_id).into());
                    CastMemberType::Unknown
                }
            }
            MemberType::Flash => {
                use crate::director::enums::ShapeType;
                debug!("Flash member {}: checking for shape_info", number);
                debug!("  specific_data has shape_info: {}", chunk.specific_data.shape_info().is_some());

                if let Some(shape_info) = chunk.specific_data.shape_info() {
                    let script_id = chunk.member_info.as_ref()
                        .map(|info| info.header.script_id)
                        .unwrap_or(0);
                    let member_script_ref = if script_id > 0 {
                        Some(CastMemberRef { cast_lib: cast_lib as i32, cast_member: script_id as i32 })
                    } else { None };
                    debug!("Flash member {} is a Shape (via shape_info), script_id={}", number, script_id);
                    return CastMember {
                        number,
                        name: chunk
                            .member_info
                            .as_ref()
                            .map(|x| x.name.to_owned())
                            .unwrap_or_default(),
                        comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                        member_type: CastMemberType::Shape(ShapeMember {
                            shape_info: shape_info.clone(),
                            script_id,
                            member_script_ref,
                        }),
                        color: ColorRef::PaletteIndex(255),
                        bg_color: ColorRef::PaletteIndex(0),
                    }
                }

                // Director MX 2004 can store shapes as Flash members
                // Try to parse the specific_data_raw as ShapeInfo
                if !chunk.specific_data_raw.is_empty() {
                    debug!("  specific_data_raw length: {}", chunk.specific_data_raw.len());

                    // Try parsing as ShapeInfo
                    let shape_info = ShapeInfo::from(chunk.specific_data_raw.as_slice());
                    debug!("  Parsed shape_type: {:?}", shape_info.shape_type);

                    // If it looks like valid shape data, treat it as a shape
                    if matches!(shape_info.shape_type, ShapeType::Rect | ShapeType::Oval | ShapeType::OvalRect | ShapeType::Line) {
                        let script_id = chunk.member_info.as_ref()
                            .map(|info| info.header.script_id)
                            .unwrap_or(0);
                        let member_script_ref = if script_id > 0 {
                            Some(CastMemberRef { cast_lib: cast_lib as i32, cast_member: script_id as i32 })
                        } else { None };
                        debug!("Flash member {} is actually a Shape! script_id={}", number, script_id);
                        return CastMember {
                            number,
                            name: chunk
                                .member_info
                                .as_ref()
                                .map(|x| x.name.to_owned())
                                .unwrap_or_default(),
                            comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                            member_type: CastMemberType::Shape(ShapeMember {
                                shape_info,
                                script_id,
                                member_script_ref,
                            }),
                            color: ColorRef::PaletteIndex(255),
                            bg_color: ColorRef::PaletteIndex(0),
                        }
                    }
                }

                // Otherwise, process as actual Flash
                web_sys::console::log_1(&format!(
                    "Flash member #{} in cast lib {}: {} children, specific_data_raw={} bytes",
                    number, cast_lib, member_def.children.len(), chunk.specific_data_raw.len()
                ).into());
                for (i, child) in member_def.children.iter().enumerate() {
                    match child {
                        Some(c) => {
                            let desc = match c {
                                Chunk::Raw(d) => format!("Raw({} bytes, sig={:?})", d.len(), &d[..3.min(d.len())]),
                                Chunk::Media(m) => format!("Media({} bytes)", m.audio_data.len()),
                                Chunk::XMedia(x) => format!("XMedia({} bytes, sig={:?})", x.raw_data.len(), &x.raw_data[..3.min(x.raw_data.len())]),
                                _ => format!("{:?}", std::mem::discriminant(c)),
                            };
                            web_sys::console::log_1(&format!("  child[{}]: {}", i, desc).into());
                        }
                        None => {
                            web_sys::console::log_1(&format!("  child[{}]: None", i).into());
                        }
                    }
                }
                if chunk.specific_data_raw.len() >= 3 {
                    web_sys::console::log_1(&format!(
                        "  specific_data_raw first 20 bytes: {:?}",
                        &chunk.specific_data_raw[..20.min(chunk.specific_data_raw.len())]
                    ).into());
                }

                // Search ALL children for SWF data (not just child 0)
                for child_opt in &member_def.children {
                    if let Some(child) = child_opt {
                        if let Some(bytes) = child.as_bytes() {
                            if let Some(cm) = Self::try_parse_swf(bytes.to_vec(), number, chunk) {
                                return cm;
                            }
                        }
                    }
                }
                // Also scan XMedia children
                if let Some(cm) = Self::scan_children_for_ole(member_def, number, chunk, bitmap_manager) {
                    return cm;
                }
                // Fallback: use first child bytes if available
                let flash_info = chunk.specific_data.flash_info().cloned();
                let reg_point = if let Some(ref info) = flash_info {
                    if info.center_reg_point {
                        (info.reg_point.0 as i16, info.reg_point.1 as i16)
                    } else {
                        (info.origin_h as i16, info.origin_v as i16)
                    }
                } else {
                    (0, 0)
                };
                if let Some(bytes) = Self::get_first_child_bytes(member_def) {
                    CastMemberType::Flash(FlashMember { data: bytes, reg_point, flash_info })
                } else {
                    warn!("Flash cast member has no data chunk or it is invalid.");
                    CastMemberType::Flash(FlashMember { data: vec![], reg_point, flash_info })
                }
            }
            MemberType::Ole => {
                Self::log_ole_start(number, cast_lib, chunk);

                // Check if this OLE member is a Shockwave3D member.
                // Format: 4-byte string length + "shockwave3d" + 3DPR data
                if Self::is_shockwave3d_ole(&chunk.specific_data_raw) {
                    let info = Shockwave3dInfo::from(&chunk.specific_data_raw)
                        .unwrap_or(Shockwave3dInfo { loops: false, duration: 0, direct_to_stage: false, animation_enabled: false, preload: false, reg_point: (0, 0), default_rect: (0, 0, 0, 0), camera_position: None, camera_rotation: None, bg_color: None, ambient_color: None });
                    let w3d_data = member_def.children.iter()
                        .filter_map(|c| c.as_ref())
                        .find_map(|c| match c {
                            Chunk::XMedia(xm) => Some(xm.raw_data.clone()),
                            Chunk::Raw(raw) => {
                                // Some Director versions store W3D data as Raw chunks
                                if raw.len() > 4 {
                                    Some(raw.clone())
                                } else {
                                    None
                                }
                            }
                            _ => None,
                        })
                        .unwrap_or_default();
                    // Dump specific_data_raw and first bytes of w3d for debugging
                    let specific_hex: Vec<String> = chunk.specific_data_raw
                        .iter().map(|b| format!("{:02X}", b)).collect();
                    // Try to decode extra 3DPR fields (camera, colors)
                    let raw = &chunk.specific_data_raw;
                    let str_len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
                    let o = 4 + str_len + 12;
                    let mut extra_info = String::new();
                    if raw.len() >= o + 0x80 {
                        // Scan for camera position/rotation floats after the known fields
                        // Try reading floats starting from o+0x57+4 onwards
                        let scan_start = o + 0x57;
                        for scan_off in (scan_start..raw.len().saturating_sub(12)).step_by(4) {
                            let f1 = f32::from_bits(u32::from_be_bytes([raw[scan_off], raw[scan_off+1], raw[scan_off+2], raw[scan_off+3]]));
                            let f2 = f32::from_bits(u32::from_be_bytes([raw[scan_off+4], raw[scan_off+5], raw[scan_off+6], raw[scan_off+7]]));
                            let f3 = f32::from_bits(u32::from_be_bytes([raw[scan_off+8], raw[scan_off+9], raw[scan_off+10], raw[scan_off+11]]));
                            // Look for plausible camera-like values
                            if f1.abs() > 0.01 && f1.abs() < 10000.0 && f2.abs() > 0.01 && f2.abs() < 10000.0 && f3.abs() > 0.01 && f3.abs() < 10000.0 {
                                extra_info.push_str(&format!("\n  @o+0x{:X}: floats ({:.4}, {:.4}, {:.4})", scan_off - o, f1, f2, f3));
                            }
                        }
                    }

                    debug!(
                        "Ole member #{} Shockwave3D: w3d={} bytes, rect=({},{},{},{})\n  specific_data_raw[{}]: {}\n  3DPR content offset=0x{:X}{}",
                        number, w3d_data.len(),
                        info.default_rect.0, info.default_rect.1, info.default_rect.2, info.default_rect.3,
                        chunk.specific_data_raw.len(), specific_hex.join(" "),
                        o, extra_info
                    );

                    // Dump the IFX view node block data if found
                    if let Some(ifx_start) = crate::director::chunks::w3d::find_ifx_start_offset(&w3d_data) {
                        let ifx = &w3d_data[ifx_start..];
                        let first256: Vec<String> = ifx[..ifx.len().min(256)].iter().map(|b| format!("{:02X}", b)).collect();
                        debug!(
                            "  IFX data at offset {}, first 256 bytes:\n  {}",
                            ifx_start, first256.join(" ")
                        );
                    }
                    let parsed_scene = if !w3d_data.is_empty() {
                        match crate::director::chunks::w3d::parse_w3d(&w3d_data) {
                            Ok(scene) => {
                                web_sys::console::log_1(&format!("W3D parsed: {} materials, {} nodes, {} meshes, {} motions",
                                    scene.materials.len(), scene.nodes.len(), scene.clod_meshes.len(), scene.motions.len()).into());
                                Some(std::rc::Rc::new(scene))
                            }
                            Err(e) => {
                                web_sys::console::error_1(&format!("W3D parse error: {}", e).into());
                                Some(std::rc::Rc::new(Self::create_empty_w3d_scene()))
                            }
                        }
                    } else {
                        // Empty W3D data — create empty scene for Lingo-created content
                        Some(std::rc::Rc::new(Self::create_empty_w3d_scene()))
                    };
                    let runtime_state = Shockwave3dRuntimeState::from_info(&info, parsed_scene.as_deref());
                    return CastMember {
                        number,
                        name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                        comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                        member_type: CastMemberType::Shockwave3d(Shockwave3dMember {
                        source_scene: None,
                        parsed_scene,
                        runtime_state,
                        info,
                        w3d_data,
                        converted_from_text: false,
                        text3d_state: None,
                        text3d_source: None,
                    }),
                        color: ColorRef::PaletteIndex(255),
                        bg_color: ColorRef::PaletteIndex(0),
                    };
                }

                // Check if this OLE member is a vectorShape by examining specific_data_raw.
                // Format: 4-byte string length + "vectorShape" + FLSH data block
                if let Some(cm) = Self::try_parse_vector_shape(&chunk.specific_data_raw, number, chunk) {
                    return cm;
                }

                // Try direct OLE data
                if let Some(bytes) = Self::get_first_child_bytes(member_def) {
                    if let Some(cm) = Self::try_parse_swf(bytes, number, chunk) {
                        return cm;
                    }
                }

                // Try all XMedia children for SWF or fonts
                if let Some(cm) = Self::scan_children_for_ole(member_def, number, chunk, bitmap_manager) {
                    return cm;
                }

                // Fallback
                Self::log_unknown_ole(number, chunk);
                CastMemberType::Unknown
            }
            MemberType::Bitmap => {
                let mut bitmap_info = chunk.specific_data.bitmap_info().unwrap().clone();

                // Adjust palette_id if offset is non-zero (Config vs MCsL numbering mismatch).
                if bitmap_info.palette_id > 0 && palette_id_offset != 0 {
                    bitmap_info.palette_id -= palette_id_offset;
                }

                let script_id = chunk
                    .member_info
                    .as_ref()
                    .map(|info| info.header.script_id)
                    .unwrap_or(0);
                
                let behavior_script_ref = if script_id > 0 {
                    let script_chunk = &lctx.as_ref().unwrap().scripts[&script_id];

                    // Create the behavior script reference
                    Some(CastMemberRef {
                        cast_lib: cast_lib as i32,
                        cast_member: script_id as i32,
                    })
                } else {
                    None
                };

                // First, check if there's a Media (ediM) chunk with JPEG data
                let media_chunk = member_def.children.iter().find_map(|c| {
                    c.as_ref().and_then(|chunk| match chunk {
                        Chunk::Media(m) => Some(m),
                        _ => None,
                    })
                });

                let new_bitmap_ref = if let Some(media) = media_chunk {
                    // Check if the media chunk contains JPEG data
                    let is_jpeg = if media.audio_data.len() >= 4 {
                        let header = u32::from_be_bytes([
                            media.audio_data[0],
                            media.audio_data[1],
                            media.audio_data[2],
                            media.audio_data[3],
                        ]);
                        // JPEG magic numbers: FFD8FFE0, FFD8FFE1, FFD8FFE2, FFD8FFDB
                        (header & 0xFFFFFF00) == 0xFFD8FF00
                    } else {
                        false
                    };

                    if is_jpeg && !media.audio_data.is_empty() {
                        // Look for ALFA chunk in children (Raw data from parsed ALFA chunk)
                        let alfa_data: Option<&Vec<u8>> = member_def.children.iter().find_map(|c| {
                            c.as_ref().and_then(|chunk| match chunk {
                                Chunk::Raw(data) => Some(data),
                                _ => None,
                            })
                        });

                        match decode_jpeg_bitmap(&media.audio_data, &bitmap_info, alfa_data) {
                            Ok(new_bitmap) => {
                                debug!(
                                    "Successfully decoded JPEG: {}x{}, bit_depth: {}",
                                    new_bitmap.width, new_bitmap.height, new_bitmap.bit_depth
                                );
                                bitmap_manager.add_bitmap(new_bitmap)
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to decode JPEG bitmap {}: {:?}. Using empty image.",
                                    number, e
                                );
                                bitmap_manager.add_bitmap(Bitmap::new(
                                    1,
                                    1,
                                    8,
                                    8,
                                    0,
                                    PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
                                ))
                            }
                        }
                    } else {
                        // Media chunk exists but doesn't contain JPEG, fall back to BITD
                        Self::decode_bitmap_from_bitd(
                            member_def,
                            &bitmap_info,
                            cast_lib,
                            number,
                            bitmap_manager,
                        )
                    }
                } else {
                    // No Media chunk, use BITD
                    Self::decode_bitmap_from_bitd(
                        member_def,
                        &bitmap_info,
                        cast_lib,
                        number,
                        bitmap_manager,
                    )
                };

                debug!(
                        "BitmapMember created â†’ name: {} palette_id {} useAlpha {} trimWhiteSpace {}",
                        chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                        bitmap_info.palette_id,
                        bitmap_info.use_alpha,
                        bitmap_info.trim_white_space
                    );

                CastMemberType::Bitmap(BitmapMember {
                    image_ref: new_bitmap_ref,
                    reg_point: (bitmap_info.reg_x, bitmap_info.reg_y),
                    script_id,
                    member_script_ref: behavior_script_ref,
                    info: bitmap_info.clone(),
                })
            }
            MemberType::Palette => {
                let palette_chunk = member_def.children.iter()
                    .find_map(|c| c.as_ref().and_then(|ch| ch.as_palette()))
                    .expect("No palette chunk found for palette member");

                CastMemberType::Palette(PaletteMember {
                    colors: palette_chunk.colors.clone(),
                })
            }
            MemberType::Shape => {
                let script_id = chunk
                    .member_info
                    .as_ref()
                    .map(|info| info.header.script_id)
                    .unwrap_or(0);

                let member_script_ref = if script_id > 0 {
                    Some(CastMemberRef {
                        cast_lib: cast_lib as i32,
                        cast_member: script_id as i32,
                    })
                } else {
                    None
                };

                web_sys::console::log_1(&format!("Shape member {} script_id={}", number, script_id).into());

                CastMemberType::Shape(ShapeMember {
                    shape_info: chunk.specific_data.shape_info().unwrap().clone(),
                    script_id,
                    member_script_ref,
                })
            }
            MemberType::FilmLoop => {
                let score_chunk_opt = member_def.children.get(0)
                    .and_then(|c| c.as_ref())
                    .and_then(|c| c.as_score());
                let film_loop_info = chunk.specific_data.film_loop_info().unwrap();

                if let Some(score_chunk) = score_chunk_opt {
                    let mut score = Score::empty();
                    score.load_from_score_chunk(score_chunk, dir_version);

                    // Compute initial_rect by finding the bounding box of all sprites
                    let initial_rect = Self::compute_filmloop_initial_rect(
                        &score_chunk.frame_data.frame_channel_data,
                        film_loop_info.reg_point,
                    );

                    debug!(
                        "FilmLoop {} initial_rect: ({}, {}, {}, {}), info size: {}x{}, reg_point: ({}, {})",
                        number,
                        initial_rect.left, initial_rect.top, initial_rect.right, initial_rect.bottom,
                        film_loop_info.width, film_loop_info.height,
                        film_loop_info.reg_point.0, film_loop_info.reg_point.1
                    );

                    // Log sprite_spans info
                    debug!(
                        "FilmLoop {} has {} sprite_spans, {} frame_intervals, {} frame_channel_data entries",
                        number,
                        score.sprite_spans.len(),
                        score_chunk.frame_intervals.len(),
                        score_chunk.frame_data.frame_channel_data.len()
                    );

                    CastMemberType::FilmLoop(FilmLoopMember {
                        info: film_loop_info.clone(),
                        score_chunk: score_chunk.clone(),
                        score,
                        current_frame: 1, // Start at frame 1
                        initial_rect,
                    })
                } else {
                    warn!("FilmLoop {} has no valid score chunk, creating empty film loop", number);
                    let empty_score_chunk = ScoreChunk {
                        header: ScoreChunkHeader {
                            total_length: 0, unk1: 0, unk2: 0,
                            entry_count: 0, unk3: 0, entry_size_sum: 0,
                        },
                        entries: vec![],
                        frame_intervals: vec![],
                        frame_data: ScoreFrameData::default(),
                        sprite_details: std::collections::HashMap::new(),
                    };
                    CastMemberType::FilmLoop(FilmLoopMember {
                        info: film_loop_info.clone(),
                        score_chunk: empty_score_chunk,
                        score: Score::empty(),
                        current_frame: 1,
                        initial_rect: super::geometry::IntRect { left: 0, top: 0, right: 0, bottom: 0 },
                    })
                }
            }
            MemberType::Sound => {
                // Log children
                if !member_def.children.is_empty() {
                    debug!(
                        "CastMember {} has {} children:",
                        number,
                        member_def.children.len()
                    );

                    for (i, c_opt) in member_def.children.iter().enumerate() {
                        match c_opt {
                            Some(c) => debug!("child[{}] = {}", i, Self::chunk_type_name(c)),
                            None => debug!("child[{}] = None", i),
                        }
                    }
                }

                // Try to find a sound chunk - check multiple formats:
                // 1. "snd " chunk (Mac snd resource)
                // 2. "ediM" chunk (MediaChunk)
                // 3. "sndH" + "sndS" chunks (Director 6+ split format)
                let sound_chunk_opt = member_def.children.iter()
                .filter_map(|c_opt| c_opt.as_ref())
                .find_map(|chunk| match chunk {
                    Chunk::Sound(s) => {
                    debug!("Found Sound chunk with {} bytes", s.data().len());
                    Some(s.clone())
                    },
                    Chunk::Media(m) => {
                    debug!("Found Media chunk: sample_rate={}, data_size_field={}, audio_data.len()={}, is_compressed={}",
                        m.sample_rate, m.data_size_field, m.audio_data.len(), m.is_compressed
                    );

                    // Check if the Media chunk has any sound data
                    // Don't just check is_empty - also check data_size_field
                    if !m.audio_data.is_empty() || m.data_size_field > 0 {
                        let sound = SoundChunk::from_media(&m);
                        debug!(
                        "Created SoundChunk from Media: {} bytes, rate={}",
                        sound.data().len(),
                        sound.sample_rate()
                        );
                        Some(sound)
                    } else {
                        debug!("Media chunk has no audio data");
                        None
                    }
                    },
                    _ => None,
                });

                // If no sound found yet, try sndH + sndS combination
                let sound_chunk_opt = if sound_chunk_opt.is_some() {
                    sound_chunk_opt
                } else {
                    // Look for sndH (header) and sndS (samples) children
                    let snd_header = member_def.children.iter()
                        .filter_map(|c| c.as_ref())
                        .find_map(|c| match c {
                            Chunk::SndHeader(h) => Some(h),
                            _ => None,
                        });
                    let snd_samples = member_def.children.iter()
                        .filter_map(|c| c.as_ref())
                        .find_map(|c| match c {
                            Chunk::SndSamples(data) => Some(data),
                            _ => None,
                        });

                    if let (Some(header), Some(samples)) = (snd_header, snd_samples) {
                        debug!(
                            "Found sndH + sndS: rate={}, bits={}, channels={}, numFrames={}, samples_len={}",
                            header.frame_rate, header.bits_per_sample, header.num_channels, header.num_frames, samples.len()
                        );
                        Some(SoundChunk::from_snd_header_and_samples(header, samples))
                    } else if let Some(_header) = snd_header {
                        debug!("Found sndH but no sndS data");
                        None
                    } else {
                        None
                    }
                };

                let found_sound = sound_chunk_opt.is_some();
                debug!(
                    "CastMember {}: {} children, found sound chunk = {}",
                    number,
                    member_def.children.len(),
                    found_sound
                );

                // Construct SoundMember
                if let Some(sound_chunk) = sound_chunk_opt {
                    let info = SoundInfo {
                        sample_rate: sound_chunk.sample_rate(),
                        sample_size: sound_chunk.bits_per_sample(),
                        channels: sound_chunk.channels(),
                        sample_count: sound_chunk.sample_count(),
                        duration: if sound_chunk.sample_rate() > 0 {
                            (sound_chunk.sample_count() as f32 / sound_chunk.sample_rate() as f32
                                * 1000.0)
                                .round() as u32
                        } else {
                            0
                        },
                        loop_enabled: chunk
                            .member_info
                            .as_ref()
                            .map_or(false, |info| (info.header.flags & 0x10) == 0),
                    };

                    debug!(
                        "SoundMember created â†’ name: {}, version: {}, sample_rate: {}, sample_size: {}, channels: {}, sample_count: {}, duration: {:.3}ms",
                        chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                        sound_chunk.version,
                        info.sample_rate,
                        info.sample_size,
                        info.channels,
                        info.sample_count,
                        info.duration
                    );

                    CastMemberType::Sound(SoundMember {
                        info,
                        sound: sound_chunk,
                    })
                } else {
                    warn!("No sound chunk found for member {}", number);
                    CastMemberType::Sound(SoundMember {
                        info: SoundInfo::default(),
                        sound: SoundChunk::default(),
                    })
                }
            }
            MemberType::Button => {
                // Button members are parsed identically to Text (FieldInfo + STXT child),
                // with an extra u16 at bytes 28-29 for button type.
                let text_chunk = member_def.children.iter()
                    .find_map(|c| c.as_ref().and_then(|ch| ch.as_text()));
                let raw = chunk.specific_data_raw.as_slice();
                let field_info = FieldInfo::from(raw);
                let mut field_member = FieldMember::from_field_info(&field_info);

                // Button dimensions come from the rect, not text_height
                field_member.width = field_info.width();
                field_member.height = field_info.height();

                // Read button type from bytes 28-29 (u16 BE, value is 1-indexed per ScummVM)
                let button_type_raw = if raw.len() >= 30 {
                    ((raw[28] as u16) << 8) | (raw[29] as u16)
                } else {
                    1 // default to pushButton (1-indexed)
                };
                let button_type = ButtonType::from_raw(button_type_raw.wrapping_sub(1));

                if let Some(text_chunk) = text_chunk {
                    field_member.text = text_chunk.text.clone();

                    let formatting_runs = text_chunk.parse_formatting_runs();
                    if let Some(first_run) = formatting_runs.first() {
                        let fg_r = (first_run.color_r >> 8) as u8;
                        let fg_g = (first_run.color_g >> 8) as u8;
                        let fg_b = (first_run.color_b >> 8) as u8;
                        field_member.fore_color = Some(ColorRef::Rgb(fg_r, fg_g, fg_b));

                        field_member.font_id = Some(first_run.font_id);
                        if let Some(font_name) = font_table.get(&first_run.font_id) {
                            field_member.font = font_name.clone();
                        }
                        if first_run.font_size > 0 {
                            field_member.font_size = first_run.font_size;
                        }
                        if first_run.height > 0 {
                            field_member.fixed_line_space = first_run.height;
                        }
                        if first_run.style != 0 {
                            let mut styles = Vec::new();
                            if (first_run.style & 0x01) != 0 { styles.push("bold"); }
                            if (first_run.style & 0x02) != 0 { styles.push("italic"); }
                            if (first_run.style & 0x04) != 0 { styles.push("underline"); }
                            if styles.is_empty() {
                                field_member.font_style = "plain".to_string();
                            } else {
                                field_member.font_style = styles.join(" ");
                            }
                        }
                    }
                }

                let script_id = chunk
                    .member_info
                    .as_ref()
                    .map(|info| info.header.script_id)
                    .unwrap_or(0);

                let behavior_script_ref = if script_id > 0 {
                    let _script_chunk = &lctx.as_ref().unwrap().scripts[&script_id];
                    Some(CastMemberRef {
                        cast_lib: cast_lib as i32,
                        cast_member: script_id as i32,
                    })
                } else {
                    None
                };

                debug!(
                    "ButtonMember text='{}' button_type={:?} font='{}' font_size={} width={} height={} script_id={}",
                    field_member.text, button_type, field_member.font, field_member.font_size,
                    field_member.width, field_member.height, script_id,
                );

                CastMemberType::Button(ButtonMember {
                    field: field_member,
                    button_type,
                    hilite: false,
                    script_id,
                    member_script_ref: behavior_script_ref,
                })
            }
            _ => {
                // Assuming `chunk.member_type` is an enum backed by a numeric ID
                // If it's not Copy, clone or cast as needed.
                let member_type_id = chunk.member_type as u16; // or u32 depending on your enum base type

                debug!(
                    "[CastMember::from] Unknown member type for member #{} (cast_lib={}): {:?} (id={})",
                    number,
                    cast_lib,
                    chunk.member_type, // this prints name, e.g. Button
                    member_type_id      // this prints numeric id, e.g. 15
                );

                if let Some(info) = &chunk.member_info {
                    debug!(
                        " name='{}', script_id={}, flags={:?}",
                        info.name, info.header.script_id, info.header.flags
                    );
                } else {
                    debug!(" No member_info available");
                }

                // Log all child chunks
                if member_def.children.is_empty() {
                    debug!(" No children found.");
                } else {
                    debug!(" {} children:", member_def.children.len());

                    for (i, c_opt) in member_def.children.iter().enumerate() {
                        match c_opt {
                            Some(c) => debug!("    child[{}] = {}", i, Self::chunk_type_name(c)),
                            None => debug!("    child[{}] = None", i),
                        }
                    }
                }

                CastMemberType::Unknown
            }
        };
        CastMember {
            number,
            name: chunk
                .member_info
                .as_ref()
                .map(|x| x.name.to_owned())
                .unwrap_or_default(),
            comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
            member_type: member_type,
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }
}
