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
    pub reg_point: (i32, i32),
}

#[derive(Clone)]
pub enum Media {
    Field(FieldMember),
    Bitmap { bitmap: Bitmap, reg_point: (i16, i16) },
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
    // Runtime selection state. Convention: sel_start <= sel_end. Equal values mean caret;
    // otherwise it's a selection range. sel_anchor remembers the drag/shift origin so
    // shift+arrow and drag-to-select extend in the right direction.
    pub sel_start: i32,
    pub sel_end: i32,
    pub sel_anchor: i32,
    // Text rendering config
    pub kerning: bool,
    pub kerning_threshold: u16,
    pub use_hypertext_styles: bool,
    pub anti_alias_type: String,
    // Cast-member-attached script (Director's "BehaviorScript" export).
    // Mirrors ButtonMember/BitmapMember/ShapeMember so Field-typed buttons
    // with a member script are recognised by `is_active_sprite` and the
    // click-transparency check, instead of having clicks silently pass
    // through them.
    pub script_id: u32,
    pub member_script_ref: Option<CastMemberRef>,
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
    /// Per-paragraph par_info table from XMED Section 0x0007. Each
    /// entry holds line_spacing (Paige `dword270`) etc. Indexed by
    /// `par_runs[N].par_info_index`. Empty for non-XMED text members.
    pub par_infos: Vec<crate::director::chunks::xmedia_styled_text::ParInfo>,
    /// Per-text-position par_info refs from XMED Section 0x0005
    /// (par_run_key). Sorted by `position`. The active par_info for a
    /// given text offset is the one whose `position` is the largest
    /// value ≤ that offset. See `line_spacing_at()` helper.
    pub par_runs: Vec<crate::director::chunks::xmedia_styled_text::ParRun>,
    pub info: Option<TextInfo>,
    /// Embedded 3D world for Director's "3D Text" feature (text extrusion).
    /// Lazily initialized when 3D methods (.model(), .camera(), etc.) are called.
    pub w3d: Option<Box<Shockwave3dMember>>,
    // Runtime selection state. Convention: sel_start <= sel_end. Equal = caret;
    // otherwise selection range. sel_anchor is the drag/shift origin.
    pub sel_start: i32,
    pub sel_end: i32,
    pub sel_anchor: i32,
    /// Anti-alias method: "AutoAlias", "GrayScaleAllAlias", "SubpixelAllAlias",
    /// "GrayscaleLargerThanAlias", or "NoneAlias".
    pub anti_alias_type: String,
    // Cast-member-attached script (Director's "BehaviorScript" export).
    // See FieldMember::script_id for the rationale.
    pub script_id: u32,
    pub member_script_ref: Option<CastMemberRef>,
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
            reg_point: (0, 0),
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
            sel_start: 0,
            sel_end: 0,
            sel_anchor: 0,
            kerning: false,
            kerning_threshold: 14,
            use_hypertext_styles: false,
            anti_alias_type: "AutoAlias".to_string(),
            script_id: 0,
            member_script_ref: None,
        }
    }

    /// Replace `text` and adjust the live caret/selection to a sensible
    /// position. When the caret was at the end of the old text, follow it
    /// to the end of the new text (handles password masks, chat-bot echoes,
    /// autocomplete, "put X into field" — all the cases where a script
    /// rewrites the text in lockstep with the user typing). Otherwise the
    /// caret stays at its byte offset, clamped to the new length, so a
    /// mid-edit refresh doesn't yank the caret to a surprising spot.
    /// Use this anywhere code mutates `field.text` outside the editor itself.
    pub fn set_text_preserving_caret(&mut self, new_text: String) {
        let old_len = self.text.len() as i32;
        let was_at_end = self.sel_start == self.sel_end && self.sel_end >= old_len;
        self.text = new_text;
        let new_len = self.text.len() as i32;
        let caret = if was_at_end { new_len } else { self.sel_end.clamp(0, new_len) };
        self.sel_start = caret;
        self.sel_end = caret;
        self.sel_anchor = caret;
    }

    pub fn from_field_info(field_info: &FieldInfo) -> FieldMember {
        let (bg_r, bg_g, bg_b) = field_info.bg_color_rgb();
        // Director's bgpal_r/g/b are QuickDraw u16 RGB values — full
        // intensity per channel = 0xFFFF, zero = 0x0000. There is no
        // palette-index encoding in this struct. Verified against figure8
        // (Batman Supersonic): white-bg fields store (0xFFFF,0xFFFF,0xFFFF),
        // black-bg fields like `timerbox` store (0x0000,0x0000,0x0000).
        // An earlier heuristic remapped all-zero bgpal to PaletteIndex(0)
        // because it was thought to mean "authored white via System-Win
        // palette index 0"; that turned out to be a misreading and made
        // genuinely-black fields render white.
        let back_color = Some(ColorRef::Rgb(bg_r, bg_g, bg_b));
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
            sel_start: 0,
            sel_end: 0,
            sel_anchor: 0,
            kerning: false,
            kerning_threshold: 14,
            use_hypertext_styles: false,
            anti_alias_type: "AutoAlias".to_string(),
            script_id: 0,
            member_script_ref: None,
        }
    }
}

impl TextMember {
    /// Replace `text` and adjust caret/selection. See FieldMember docs.
    pub fn set_text_preserving_caret(&mut self, new_text: String) {
        let old_len = self.text.len() as i32;
        let was_at_end = self.sel_start == self.sel_end && self.sel_end >= old_len;
        self.text = new_text;
        let new_len = self.text.len() as i32;
        let caret = if was_at_end { new_len } else { self.sel_end.clamp(0, new_len) };
        self.sel_start = caret;
        self.sel_end = caret;
        self.sel_anchor = caret;
    }

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
            par_infos: Vec::new(),
            par_runs: Vec::new(),
            info: None,
            w3d: None,
            sel_start: 0,
            sel_end: 0,
            sel_anchor: 0,
            anti_alias_type: "AutoAlias".to_string(),
            script_id: 0,
            member_script_ref: None,
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

        // Apply directionalPreset to light node transform (3D Z-up version)
        if let Some(ti) = ti {
            if ti.directional_preset > 0 && ti.directional_preset <= 9 {
                if let Some(light_node) = scene.nodes.iter_mut().find(|n| n.name == "DefaultDirectional") {
                    light_node.transform = Self::directional_preset_to_transform_3d(ti.directional_preset);
                }
            }
        }

        // Set camera position from TextInfo.
        // When TextInfo stores x=0,y=0 (default), Director auto-computes the camera
        // to center the text box in the viewport using the default FOV and aspect ratio.
        let default_fov = 34.516_f32;
        let cam_pos: Option<(f32, f32, f32)> = ti
            .map(|i| {
                let (mut px, mut py, mut pz) = (i.camera_position_x, i.camera_position_y, i.camera_position_z);
                if px == 0.0 && py == 0.0 {
                    // Auto-compute: center camera on text box
                    let w = self.width as f32;
                    let h = self.height as f32;
                    let aspect = w / h.max(1.0);
                    let v_fov_rad = (default_fov / 2.0).to_radians();
                    let h_half_fov = (v_fov_rad.tan() * aspect).atan();
                    px = w / 2.0;
                    py = h / 2.0;
                    pz = (w / 2.0) / h_half_fov.tan();
                }
                (px, py, pz)
            })
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
    /// from a directionalPreset value (1-9).
    ///
    /// The mesh front-face normal is (0,0,-1) and edge normals are inverted
    /// (top edge = (0,-1,0), etc.) because of face winding conventions.
    /// L (surface-to-light) must have matching signs for dot(N,L)>0.
    pub(crate) fn directional_preset_to_transform(preset: u32) -> [f32; 16] {
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

    /// 3D directional preset: Z-up world space.
    /// "top" = +Z, "left" = -X, light has forward component toward -Y.
    pub(crate) fn directional_preset_to_transform_3d(preset: u32) -> [f32; 16] {
        let (lx, lz): (f32, f32) = match preset {
            1 => (-1.0,  1.0), 2 => ( 0.0,  1.0), 3 => ( 1.0,  1.0),
            4 => (-1.0,  0.0), 5 => ( 0.0,  0.0), 6 => ( 1.0,  0.0),
            7 => (-1.0, -1.0), 8 => ( 0.0, -1.0), 9 => ( 1.0, -1.0),
            _ => return [1.0,0.0,0.0,0.0, 0.0,1.0,0.0,0.0, 0.0,0.0,1.0,0.0, 0.0,0.0,0.0,1.0],
        };
        let ly: f32 = -0.75;
        let len = (lx*lx + ly*ly + lz*lz).sqrt();
        let l = [lx/len, ly/len, lz/len];
        let z = [l[0], l[1], l[2]];
        let up = if z[2].abs() < 0.9 { [0.0,0.0,1.0] } else { [0.0,1.0,0.0] };
        let mut x = [up[1]*z[2]-up[2]*z[1], up[2]*z[0]-up[0]*z[2], up[0]*z[1]-up[1]*z[0]];
        let xl = (x[0]*x[0]+x[1]*x[1]+x[2]*x[2]).sqrt();
        x = [x[0]/xl, x[1]/xl, x[2]/xl];
        let y = [z[1]*x[2]-z[2]*x[1], z[2]*x[0]-z[0]*x[2], z[0]*x[1]-z[1]*x[0]];
        [x[0],x[1],x[2],0.0, y[0],y[1],y[2],0.0, z[0],z[1],z[2],0.0, 0.0,0.0,0.0,1.0]
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

// `VectorShapeVertex` is defined in `crate::director::enums` and re-exported
// here so existing call sites (rasterizer, Lingo handlers) can continue to
// import it from `cast_member::*`. The FLSH payload parser is also there
// (as `VectorShapeInfo::from(&[u8])`) — the rest of player code keeps using
// this flat `VectorShapeMember` (which adds the runtime-computed bbox on
// top of the parsed `VectorShapeInfo`).
pub use crate::director::enums::VectorShapeVertex;

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
    /// Bounding box computed from vertices + control points + stroke padding.
    /// Used by the rasterizer / image getter — Director's `member.width` and
    /// `member.height` come from `member_width` / `member_height` (the FLSH
    /// header's authored values), not this.
    pub bbox_left: f32,
    pub bbox_top: f32,
    pub bbox_right: f32,
    pub bbox_bottom: f32,
    /// Authored member rect from the FLSH header (offsets 0x20 / 0x24).
    /// `member.rect`, `.width`, `.height` should come from these — they
    /// match Director's report exactly, while a vertex-derived bbox is off
    /// by ~1 due to stroke padding rounding.
    pub member_width: u32,
    pub member_height: u32,
    // ---- Display / fill / origin properties surfaced via Lingo getters.
    // Confirmed offsets from triangulating two FLSH payloads (figure8
    // members #9 and #13 / Slider Groove). Enum / bool fields are still
    // mapped to defaults until a third payload disambiguates them.
    pub reg_point: (i16, i16),    // FLSH 0x14 / 0x10  (x / y)
    pub gradient_type: String,    // default "linear"  (FLSH offset TBD)
    pub fill_scale: f32,          // FLSH 0x38, default 100.0
    pub fill_direction: f32,      // FLSH 0x3C, degrees, default 0.0
    pub fill_offset: (i32, i32),  // FLSH 0x40 / 0x44, default (0, 0)
    pub fill_cycles: i32,         // default 1  (FLSH offset TBD)
    pub scale_mode: String,       // default "autoSize"  (FLSH offset TBD)
    pub scale: f32,               // FLSH 0x50, percent, default 100.0
    pub antialias: bool,          // default true  (FLSH offset TBD)
    pub center_reg_point: bool,   // default false (FLSH offset TBD)
    pub reg_point_vertex: i32,    // default 0     (FLSH offset TBD)
    pub direct_to_stage: bool,    // default false (FLSH offset TBD)
    pub origin_mode: String,      // default "center" (FLSH offset TBD)
}

impl VectorShapeMember {
    /// Director's `member.width` for vector shapes — the authored member
    /// width from the FLSH header (offset 0x24). We previously derived this
    /// from the vertex bounding box, which was off by 1 due to stroke
    /// padding. Falls back to bbox_right - bbox_left if member_width is 0
    /// (synthesized members from Lingo `new(#vectorShape)`).
    pub fn width(&self) -> f32 {
        if self.member_width > 0 {
            self.member_width as f32
        } else {
            self.bbox_right - self.bbox_left
        }
    }
    pub fn height(&self) -> f32 {
        if self.member_height > 0 {
            self.member_height as f32
        } else {
            self.bbox_bottom - self.bbox_top
        }
    }
    /// The vertex-bounding-box dimensions. Used by the rasterizer to size
    /// the bitmap that backs `member.image`.
    pub fn bbox_width(&self) -> f32 { self.bbox_right - self.bbox_left }
    pub fn bbox_height(&self) -> f32 { self.bbox_bottom - self.bbox_top }
}

impl From<crate::director::enums::VectorShapeInfo> for VectorShapeMember {
    /// Build a runtime `VectorShapeMember` from a parsed FLSH
    /// `VectorShapeInfo`. The static fields are copied over verbatim; the
    /// (bbox_left, bbox_top, bbox_right, bbox_bottom) bounding box is
    /// computed here from the vertices + Bezier control points + stroke
    /// padding (used by the rasterizer to size the off-screen bitmap that
    /// backs `member.image`).
    fn from(info: crate::director::enums::VectorShapeInfo) -> Self {
        let mut bbox_left = f32::MAX;
        let mut bbox_top = f32::MAX;
        let mut bbox_right = f32::MIN;
        let mut bbox_bottom = f32::MIN;
        for v in &info.vertices {
            bbox_left = bbox_left.min(v.x);
            bbox_top = bbox_top.min(v.y);
            bbox_right = bbox_right.max(v.x);
            bbox_bottom = bbox_bottom.max(v.y);
            // include absolute control points (vertex + handle offsets)
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
        let pad = info.stroke_width / 2.0;
        bbox_left -= pad;
        bbox_top -= pad;
        bbox_right += pad;
        bbox_bottom += pad;
        if info.vertices.is_empty() {
            bbox_left = 0.0;
            bbox_top = 0.0;
            bbox_right = 0.0;
            bbox_bottom = 0.0;
        }
        VectorShapeMember {
            stroke_color: info.stroke_color,
            fill_color: info.fill_color,
            bg_color: info.bg_color,
            end_color: info.end_color,
            stroke_width: info.stroke_width,
            fill_mode: info.fill_mode,
            closed: info.closed,
            vertices: info.vertices,
            bbox_left,
            bbox_top,
            bbox_right,
            bbox_bottom,
            member_width: info.member_width,
            member_height: info.member_height,
            reg_point: info.reg_point,
            gradient_type: info.gradient_type,
            fill_scale: info.fill_scale,
            fill_direction: info.fill_direction,
            fill_offset: info.fill_offset,
            fill_cycles: info.fill_cycles,
            scale_mode: info.scale_mode,
            scale: info.scale,
            antialias: info.antialias,
            center_reg_point: info.center_reg_point,
            reg_point_vertex: info.reg_point_vertex,
            direct_to_stage: info.direct_to_stage,
            origin_mode: info.origin_mode,
        }
    }
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
    /// Cached total frame count (computed once from channel_initialization_data, sprite_spans, and keyframes).
    pub cached_total_frames: Option<u32>,
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

impl FlashMember {
    /// Returns the effective stage rect (left, top, right, bottom) for this
    /// Flash member. Director caches the rect in the FLSH chunk's FlashInfo,
    /// but for some members it's left as 0,0,0,0 — fall back to parsing the
    /// SWF header so width/height aren't reported as zero.
    pub fn effective_rect(&self) -> (i32, i32, i32, i32) {
        let cached = self.flash_info.as_ref().map(|fi| fi.flash_rect).unwrap_or((0, 0, 0, 0));
        if cached != (0, 0, 0, 0) {
            return cached;
        }
        if let Some((w, h)) = CastMember::parse_swf_dimensions(&self.data) {
            return (0, 0, w as i32, h as i32);
        }
        cached
    }
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

    // ─── pointAtOrientation per node ───
    /// Per-node pointAtOrientation: node_name -> (front_axis, up_axis)
    /// Default: ([0,0,1], [0,1,0]) — +Z front, +Y up
    pub point_at_orientations: std::collections::HashMap<String, ([f32; 3], [f32; 3])>,

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

    // ─── Directional light preset ───
    /// member.directionalPreset value (1-9 matching topLeft..bottomRight,
    /// 0 = #None, 2 = #topCenter = Director default).
    pub directional_preset: u32,

    // ─── userData per node (Director chapter 15: model.userData / group.userData) ───
    /// Per-node `.userData` PropList datum-ref. Lazily allocated on first
    /// access — `model.userData` returns the cached DatumRef so subsequent
    /// `setaProp` / `addProp` mutations on the returned PropList stay
    /// visible across reads. Keyed by node name (case-insensitive lookup
    /// done at access time).
    pub user_data: std::collections::HashMap<String, crate::player::DatumRef>,

    // ─── W3D event/timer registrations (registerForEvent / unregisterAllEvents) ───
    /// Per-member event subscriptions. Currently only `#timeMS` is honoured
    /// by the dispatcher; collision/animation events are stored but never
    /// fired (their producers aren't wired up). `unregisterAllEvents`
    /// truncates this Vec.
    pub registered_events: Vec<RegisteredW3dEvent>,
}

#[derive(Clone, Debug)]
pub struct RegisteredW3dEvent {
    /// `#timeMS`, `#collideAny`, `#collideWith`, `#animationStarted`,
    /// `#animationEnded`, or any user-defined symbol.
    pub event_name: String,
    pub handler_name: String,
    /// Script instance to dispatch the handler on. `None` corresponds to
    /// passing `0` for `scriptObject` in Lingo — Director then searches
    /// movie scripts for the handler.
    pub script_instance: Option<crate::player::script_ref::ScriptInstanceRef>,
    /// Director's `begin`: ms after registration before the first fire.
    pub begin_ms: u32,
    /// Director's `period`: ms between fires when `repetitions > 1` or 0
    /// (infinite). Ignored for non-#timeMS events.
    pub period_ms: u32,
    /// 0 means infinite. Otherwise the total number of fires.
    pub repetitions: u32,
    /// Wall-clock ms when registerForEvent was called.
    pub registered_at_ms: f64,
    /// Number of times this event has fired so far.
    pub fires_so_far: u32,
    /// Wall-clock ms of the most recent fire (or `registered_at_ms` if
    /// not yet fired). Drives the inter-fire delta computation passed
    /// to the handler.
    pub last_fire_ms: f64,
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
            directional_preset: 2, // Director default: #topCenter
            ..Default::default()
        };
        // Seed camera transform from 3DPR camera position/rotation.
        // When stored position has x=0 and y=0, Director auto-computes the camera
        // to center the member's default_rect in the viewport using the default FOV.
        let camera_position = info.camera_position.map(|(px, py, pz)| {
            if px == 0.0 && py == 0.0 {
                let w = (info.default_rect.2 - info.default_rect.0) as f32;
                let h = (info.default_rect.3 - info.default_rect.1) as f32;
                if w > 0.0 && h > 0.0 {
                    let default_fov = 34.516_f32;
                    let aspect = w / h;
                    let h_half_fov = ((default_fov / 2.0).to_radians().tan() * aspect).atan();
                    ((w / 2.0), (h / 2.0), (w / 2.0) / h_half_fov.tan())
                } else {
                    (px, py, pz)
                }
            } else {
                (px, py, pz)
            }
        });
        if let Some((px, py, pz)) = camera_position {
            let (rx, ry, rz) = info.camera_rotation.unwrap_or((0.0, 0.0, 0.0));
            // Build camera transform from position + Euler rotation (degrees)
            // Rotation order: R = Rz * Ry * Rx
            let rx_rad = rx.to_radians();
            let ry_rad = ry.to_radians();
            let rz_rad = rz.to_radians();
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
            // Insert only under the scene node name (case-insensitive lookups handle the rest)
            let cam_key = scene.map(|s| {
                s.nodes.iter()
                    .find(|n| n.node_type == crate::director::chunks::w3d::types::W3dNodeType::View
                        && n.name.eq_ignore_ascii_case("defaultview"))
                    .map(|n| n.name.clone())
            }).flatten().unwrap_or_else(|| "DefaultView".to_string());
            state.node_transforms.insert(cam_key, m);
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

// ---- Havok Physics Member ----



// RapierWorld and HavokCollisionFilter removed — replaced by native Havok physics

#[derive(Clone, Debug)]
pub struct HavokCorrector {
    pub enabled: bool,
    pub threshold: f64,
    pub multiplier: f64,
    pub level: i32,
    pub max_tries: i32,
    pub max_distance: f64,
}

impl Default for HavokCorrector {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: 0.1,
            multiplier: 5.0,
            level: 2,
            max_tries: 10,
            max_distance: 100.0,
        }
    }
}

#[derive(Clone, Debug)]
pub struct RestingContact {
    pub normal: [f64; 3],
    pub plane_point: [f64; 3],
    pub ground_friction: f64,
    pub aabb_min: [f64; 3],
    pub aabb_max: [f64; 3],
}

#[derive(Clone, Debug)]
pub struct HavokRigidBody {
    pub name: String,
    pub position: [f64; 3],
    pub rotation_axis: [f64; 3],
    pub rotation_angle: f64,
    pub mass: f64,
    pub restitution: f64,
    pub friction: f64,
    pub active: bool,
    pub pinned: bool,
    pub linear_velocity: [f64; 3],
    pub angular_velocity: [f64; 3],
    pub linear_momentum: [f64; 3],
    pub angular_momentum: [f64; 3],
    pub force: [f64; 3],
    pub torque: [f64; 3],
    pub center_of_mass: [f64; 3],
    pub corrector: HavokCorrector,
    pub is_fixed: bool,
    pub is_convex: bool,
    /// Half-extents of the mesh bounding box, for box inertia computation.
    pub inertia_half_extents: [f64; 3],
    // --- Full Havok engine state (ported from C# reference) ---
    /// Orientation quaternion [w, x, y, z]. Internal physics state.
    pub orientation: [f64; 4],
    /// Inverse mass (0 if pinned/fixed).
    pub inverse_mass: f64,
    /// 3x3 inertia tensor (row-major). I = UnitInertiaTensor * mass.
    pub inertia_tensor: [f64; 9],
    /// 3x3 inverse inertia tensor (row-major).
    pub inverse_inertia_tensor: [f64; 9],
    /// Mass-independent unit inertia tensor.
    pub unit_inertia_tensor: [f64; 9],
    /// Saved state for bisection rollback.
    pub saved_position: [f64; 3],
    pub saved_orientation: [f64; 4],
    pub saved_linear_velocity: [f64; 3],
    pub saved_angular_velocity: [f64; 3],
    /// Resting contact: surface normal, plane point, ground friction, mesh AABB.
    /// Set on first collision with a rigid-body-owned mesh. Used each
    /// substep to keep the body on the surface analytically.
    pub resting_normal: Option<RestingContact>,
}

impl HavokRigidBody {
    pub fn new_movable(name: &str, mass: f64, is_convex: bool) -> Self {
        // Placeholder isotropic box inertia for a 20×20×20 AABB.
        // Callers that know the real mesh geometry must overwrite
        // `unit_inertia_tensor` and then call `recompute_body_inertia`
        // — see `make_movable_rigid_body` which runs the Mirtich
        // polyhedron integrator (PPC InertialTensorComputer 0x5d3c0).
        let d = 20.0_f64;
        let f = 1.0 / 12.0;
        let unit_diag = f * (d*d + d*d); // (dy²+dz²)/12 with dx=dy=dz
        let unit_i = [unit_diag, 0.0, 0.0, 0.0, unit_diag, 0.0, 0.0, 0.0, unit_diag];
        let i_tensor = [unit_i[0]*mass, 0.0, 0.0, 0.0, unit_i[4]*mass, 0.0, 0.0, 0.0, unit_i[8]*mass];
        let inv_i = if mass > 0.0 && unit_diag > 0.0 {
            [1.0/i_tensor[0], 0.0, 0.0, 0.0, 1.0/i_tensor[4], 0.0, 0.0, 0.0, 1.0/i_tensor[8]]
        } else { [0.0; 9] };
        Self {
            name: name.to_string(),
            position: [0.0; 3],
            rotation_axis: [0.0, 1.0, 0.0],
            rotation_angle: 0.0,
            mass,
            restitution: 0.3,
            friction: 0.5,
            active: true,
            pinned: false,
            linear_velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            linear_momentum: [0.0; 3],
            angular_momentum: [0.0; 3],
            force: [0.0; 3],
            torque: [0.0; 3],
            center_of_mass: [0.0; 3],
            corrector: HavokCorrector::default(),
            is_fixed: false,
            is_convex,
            inertia_half_extents: [10.0; 3],
            orientation: [1.0, 0.0, 0.0, 0.0],
            inverse_mass: if mass > 0.0 { 1.0 / mass } else { 0.0 },
            inertia_tensor: i_tensor,
            inverse_inertia_tensor: inv_i,
            unit_inertia_tensor: unit_i,
            saved_position: [0.0; 3],
            saved_orientation: [1.0, 0.0, 0.0, 0.0],
            saved_linear_velocity: [0.0; 3],
            saved_angular_velocity: [0.0; 3],
            resting_normal: None,
        }
    }

    pub fn new_fixed(name: &str, is_convex: bool) -> Self {
        Self {
            name: name.to_string(),
            position: [0.0; 3],
            rotation_axis: [0.0, 1.0, 0.0],
            rotation_angle: 0.0,
            mass: 0.0,
            restitution: 0.3,
            friction: 0.5,
            active: false,
            pinned: true,
            linear_velocity: [0.0; 3],
            angular_velocity: [0.0; 3],
            linear_momentum: [0.0; 3],
            angular_momentum: [0.0; 3],
            force: [0.0; 3],
            torque: [0.0; 3],
            center_of_mass: [0.0; 3],
            corrector: HavokCorrector::default(),
            is_fixed: true,
            is_convex,
            inertia_half_extents: [10.0; 3],
            orientation: [1.0, 0.0, 0.0, 0.0],
            inverse_mass: 0.0,
            inertia_tensor: [0.0; 9],
            inverse_inertia_tensor: [0.0; 9],
            unit_inertia_tensor: [0.0; 9],
            saved_position: [0.0; 3],
            saved_orientation: [1.0, 0.0, 0.0, 0.0],
            saved_linear_velocity: [0.0; 3],
            saved_angular_velocity: [0.0; 3],
            resting_normal: None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HavokSpring {
    pub name: String,
    pub rigid_body_a: Option<String>,
    pub rigid_body_b: Option<String>,
    pub point_a: [f64; 3],
    pub point_b: [f64; 3],
    pub rest_length: f64,
    pub elasticity: f64,
    pub damping: f64,
    pub on_compression: bool,
    pub on_extension: bool,
}

impl HavokSpring {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            rigid_body_a: None,
            rigid_body_b: None,
            point_a: [0.0; 3],
            point_b: [0.0; 3],
            rest_length: 0.0,
            elasticity: 0.5,
            damping: 0.1,
            on_compression: true,
            on_extension: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HavokLinearDashpot {
    pub name: String,
    pub rigid_body_a: Option<String>,
    pub rigid_body_b: Option<String>,
    pub point_a: [f64; 3],
    pub point_b: [f64; 3],
    pub strength: f64,
    pub damping: f64,
}

impl HavokLinearDashpot {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            rigid_body_a: None,
            rigid_body_b: None,
            point_a: [0.0; 3],
            point_b: [0.0; 3],
            strength: 0.5,
            damping: 0.1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HavokAngularDashpot {
    pub name: String,
    pub rigid_body_a: Option<String>,
    pub rigid_body_b: Option<String>,
    pub rotation_axis: [f64; 3],
    pub rotation_angle: f64,
    pub strength: f64,
    pub damping: f64,
}

impl HavokAngularDashpot {
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            rigid_body_a: None,
            rigid_body_b: None,
            rotation_axis: [0.0, 1.0, 0.0],
            rotation_angle: 0.0,
            strength: 0.5,
            damping: 0.1,
        }
    }
}

#[derive(Clone, Debug)]
pub struct HavokCollisionInterest {
    pub rb_name1: String,
    pub rb_name2: String,  // or "#all"
    pub frequency: f64,
    pub threshold: f64,
    pub handler_name: Option<String>,
    pub script_instance: Option<crate::player::DatumRef>,
}

/// Collision contact info for the collisionList property
#[derive(Clone, Debug)]
pub struct HavokCollisionInfo {
    pub body_a: String,
    pub body_b: String,
    pub point: [f64; 3],
    pub normal: [f64; 3],
}

pub struct HavokPhysicsState {
    pub initialized: bool,
    pub w3d_cast_lib: i32,
    pub w3d_cast_member: i32,
    pub tolerance: f64,
    pub scale: f64,
    pub gravity: [f64; 3],
    pub sim_time: f64,
    pub time_step: f64,
    pub sub_steps: i32,
    pub deactivation_params: [f64; 2],
    pub drag_params: [f64; 2],
    pub rigid_bodies: Vec<HavokRigidBody>,
    pub springs: Vec<HavokSpring>,
    pub linear_dashpots: Vec<HavokLinearDashpot>,
    pub angular_dashpots: Vec<HavokAngularDashpot>,
    pub collision_interests: Vec<HavokCollisionInterest>,
    pub step_callbacks: Vec<(String, crate::player::DatumRef)>,  // (handler_name, script_instance)
    pub disabled_collision_pairs: Vec<(String, String)>,
    pub hke_data: Vec<u8>,
    /// Ground Z for native Havok ground constraint (flat plane fallback)
    pub ground_z: f64,
    /// Half-extent Z for ground collision (car body extends this far below position)
    pub ground_body_half_z: f64,
    /// Collision meshes from HKE data (positioned in world space)
    pub collision_meshes: Vec<crate::player::handlers::datum_handlers::cast_member::havok_physics::CollisionMesh>,
    /// Cached collision list from last step (for Lingo collisionList property)
    pub collision_list_cache: Vec<HavokCollisionInfo>,
    /// Whether to use the raycast-based ground constraint hack (SuperSonic-specific).
    /// Disabled for HKE scene games where bodies are auto-created from W3D nodes,
    /// because those scenes need proper GJK collision instead of a simple ground clamp.
    pub use_ground_constraint: bool,
}

impl Clone for HavokPhysicsState {
    fn clone(&self) -> Self {
        Self {
            initialized: self.initialized,
            w3d_cast_lib: self.w3d_cast_lib,
            w3d_cast_member: self.w3d_cast_member,
            tolerance: self.tolerance,
            scale: self.scale,
            gravity: self.gravity,
            sim_time: self.sim_time,
            time_step: self.time_step,
            sub_steps: self.sub_steps,
            deactivation_params: self.deactivation_params,
            drag_params: self.drag_params,
            rigid_bodies: self.rigid_bodies.clone(),
            springs: self.springs.clone(),
            linear_dashpots: self.linear_dashpots.clone(),
            angular_dashpots: self.angular_dashpots.clone(),
            collision_interests: self.collision_interests.clone(),
            step_callbacks: self.step_callbacks.clone(),
            disabled_collision_pairs: self.disabled_collision_pairs.clone(),
            hke_data: self.hke_data.clone(),
            ground_z: self.ground_z,
            ground_body_half_z: self.ground_body_half_z,
            collision_meshes: Vec::new(), // Not cloned — rebuilt on initialize
            collision_list_cache: Vec::new(),
            use_ground_constraint: self.use_ground_constraint,
        }
    }
}

impl fmt::Debug for HavokPhysicsState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HavokPhysicsState")
            .field("initialized", &self.initialized)
            .field("rigid_bodies", &self.rigid_bodies.len())
            .finish()
    }
}

impl Default for HavokPhysicsState {
    fn default() -> Self {
        Self {
            initialized: false,
            w3d_cast_lib: 0,
            w3d_cast_member: 0,
            tolerance: 0.1,
            scale: 0.0254,
            gravity: [0.0, 0.0, -386.22],
            sim_time: 0.0,
            time_step: 1.0 / 60.0,
            sub_steps: 4,
            deactivation_params: [2.0, 0.1],
            drag_params: [0.0, 0.0],
            rigid_bodies: Vec::new(),
            springs: Vec::new(),
            linear_dashpots: Vec::new(),
            angular_dashpots: Vec::new(),
            collision_interests: Vec::new(),
            step_callbacks: Vec::new(),
            disabled_collision_pairs: Vec::new(),
            hke_data: Vec::new(),
            ground_z: -1e20,
            ground_body_half_z: 8.0,
            collision_meshes: Vec::new(),
            collision_list_cache: Vec::new(),
            use_ground_constraint: true,
        }
    }
}

#[derive(Clone)]
pub struct HavokPhysicsMember {
    pub state: HavokPhysicsState,
}

impl fmt::Debug for HavokPhysicsMember {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "HavokPhysicsMember(initialized={}, bodies={}, springs={})",
            self.state.initialized,
            self.state.rigid_bodies.len(),
            self.state.springs.len())
    }
}

impl HavokPhysicsMember {
    pub fn new(hke_data: Vec<u8>) -> Self {
        let mut state = HavokPhysicsState::default();
        state.hke_data = hke_data;
        Self { state }
    }
}

// ---- PhysX (AGEIA) Physics Member ----
//
// Direct Rust port of the C# AGEIA wrapper layer in
// `E:\Claude Project\PhysXReference\`. Mirrors how Havok is wired above.
//
// PhysX members carry NO authored binary state on disk — they're
// MemberType::Ole with PROGID "Physics" and an empty children list.
// All world state is constructed at runtime via Lingo calls.
//
// Property names align with the Director Physics Xtra reference docs
// (E:\Documents\Director\director_reference.md ch. 15).

/// A single rigid body in the PhysX world. Mirrors the C# wrapper class
/// `CPhysicsRigidBodyAGEIA` plus its Director-visible cached state.
#[derive(Debug, Clone)]
pub struct PhysXRigidBody {
    pub id: u32,
    pub name: String,
    pub model_name: String,
    pub body_type: PhysXBodyType,           // Static / Dynamic / Kinematic
    pub shape: PhysXShapeKind,              // box / sphere / convex / concave
    pub position: [f64; 3],
    /// Axis-angle: (axis.x, axis.y, axis.z, angleDeg). Director's
    /// `Orientation` type — NOT a quaternion despite the typedef.
    pub orientation: [f64; 4],
    pub linear_velocity: [f64; 3],
    pub angular_velocity: [f64; 3],
    pub mass: f64,
    pub center_of_mass: [f64; 3],
    pub use_center_of_mass: bool,
    pub friction: f64,
    pub restitution: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub sleep_threshold: f64,
    pub sleep_mode: i32,
    pub collision_group: i32,
    pub is_trigger: bool,
    pub ccd_enabled: bool,
    pub pinned: bool,                       // Director "isPinned" (chapter 15)
    pub axis_affinity: bool,                // Director "axisAffinity"; default true
    pub cached_is_sleeping: bool,           // mirrors C# byte +176 cache
    pub user_data: i32,                     // opaque pointer-equivalent
    pub constraint_ids: Vec<u32>,
    // ---- Shape dimensions ----
    // Populated by createRigidBody from the Lingo-side shape parameters
    // (Director chapter 15: `#box`, `#sphere`, `#convexShape`, etc.). Read
    // by the Gu* narrowphase. Defaults are 1.0 sphere when no info given.
    pub radius: f64,                        // sphere / capsule
    pub half_extents: [f64; 3],             // box (and convex/concave AABB fallback)
    pub half_height: f64,                   // capsule (axis along body local +X)

    /// Convex hull data — set by `setConvexHull(verts, faces)` on the
    /// rigid-body Datum, consumed by the convex narrowphase. None until
    /// a script populates it (or the cooked mesh gets wired in later).
    pub convex_hull: Option<crate::player::handlers::datum_handlers::cast_member::physx_gu_convex::PolygonalData>,

    /// Triangle-mesh data for #concaveShape bodies. Set by
    /// `setTriangleMesh(verts, triangles)` on the rigid-body Datum, consumed
    /// by the mesh-vs-shape narrowphase in `physx_gu_mesh`. None until a
    /// script populates it (typical: `createConcaveMesh` ⇒ assign verts/faces).
    pub triangle_mesh: Option<crate::player::handlers::datum_handlers::cast_member::physx_gu_mesh::GuTriangleMesh>,
}

impl Default for PhysXRigidBody {
    fn default() -> Self {
        Self {
            id: 0, name: String::new(), model_name: String::new(),
            body_type: PhysXBodyType::Dynamic,
            shape: PhysXShapeKind::Box,
            position: [0.0; 3], orientation: [1.0, 0.0, 0.0, 0.0],
            linear_velocity: [0.0; 3], angular_velocity: [0.0; 3],
            mass: 1.0, center_of_mass: [0.0; 3], use_center_of_mass: false,
            friction: 0.5, restitution: 0.5,
            linear_damping: 0.0, angular_damping: 0.0,
            sleep_threshold: 0.0316, sleep_mode: 0,
            collision_group: 0, is_trigger: false, ccd_enabled: false,
            pinned: false, axis_affinity: true, cached_is_sleeping: false,
            user_data: 0, constraint_ids: Vec::new(),
            radius: 1.0, half_extents: [0.5, 0.5, 0.5], half_height: 1.0,
            convex_hull: None,
            triangle_mesh: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysXBodyType { Static, Dynamic, Kinematic }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysXShapeKind { Box, Sphere, Capsule, ConvexShape, ConcaveShape }

/// Joints + springs share one Vec keyed by ID; the variant tells us which.
/// Mirrors the C# `CPhysicsConstraintAGEIA` hierarchy.
#[derive(Debug, Clone)]
pub struct PhysXConstraint {
    pub id: u32,
    pub name: String,
    pub kind: PhysXConstraintKind,
    pub body_a: Option<u32>,                 // body id
    pub body_b: Option<u32>,
    pub anchor_a: [f64; 3],
    pub anchor_b: [f64; 3],
    pub stiffness: f64,
    pub damping: f64,
    pub rest_length: f64,
    /// Axis-angle (LinearJoint stores its own orientation; D6 has axis-angle drive too).
    pub orientation: [f64; 4],
    /// D6-only: per-axis motion[XYZ TwistSwing1Swing2] (0=Locked / 1=Limited / 2=Free).
    pub d6_linear_motion: [u8; 3],
    pub d6_angular_motion: [u8; 3],
}

impl Default for PhysXConstraint {
    fn default() -> Self {
        Self {
            id: 0, name: String::new(),
            kind: PhysXConstraintKind::Spring,
            body_a: None, body_b: None,
            anchor_a: [0.0; 3], anchor_b: [0.0; 3],
            stiffness: 0.0, damping: 0.0, rest_length: 0.0,
            orientation: [1.0, 0.0, 0.0, 0.0],
            d6_linear_motion: [0; 3], d6_angular_motion: [0; 3],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysXConstraintKind { Spring, LinearJoint, AngularJoint, D6Joint }

/// A static heightfield terrain. Director chapter 15: created via
/// `world.createTerrain(name, desc, position, orientation, rowScale,
/// columnScale, heightScale)`. The terrain is treated as a static-only
/// collider — it never integrates and never receives impulses.
#[derive(Debug, Clone)]
pub struct PhysXTerrain {
    pub id: u32,
    pub name: String,
    pub height_field: crate::player::handlers::datum_handlers::cast_member::physx_gu_heightfield::GuHeightField,
    pub friction: f64,
    pub restitution: f64,
    /// World-space position offset applied to the heightfield's local origin.
    pub position: [f64; 3],
    /// Axis-angle (axis.x, axis.y, axis.z, angle_deg) — same convention as
    /// `PhysXRigidBody.orientation`.
    pub orientation: [f64; 4],
}

/// The full simulation world. Direct port of the C#
/// `CPhysicsWorldAGEIA` field set, minus shape/cooking helpers.
#[derive(Debug, Clone)]
pub struct PhysXPhysicsState {
    // ---- World props (Lingo getters/setters) ----
    pub initialized: bool,                    // Lingo `isInitialized`
    pub paused: bool,
    pub gravity: [f64; 3],
    pub friction: f64,
    pub restitution: f64,
    pub linear_damping: f64,
    pub angular_damping: f64,
    pub contact_tolerance: f64,
    pub sleep_threshold: f64,
    pub sleep_mode: PhysXSleepMode,
    pub scaling_factor: [f64; 3],             // length, mass, time scales
    pub time_step: f64,
    pub time_step_mode: PhysXTimeStepMode,
    pub sub_steps: u32,                       // Director: `subSteps` (NOT `subStepCount`)
    pub sim_time: f64,                        // accumulated sim time → getSimulationTime()
    pub three_d_member_name: String,          // associated 3D world member

    // ---- Containers ----
    pub bodies: Vec<PhysXRigidBody>,
    pub constraints: Vec<PhysXConstraint>,    // joints + springs (kind tags)
    /// Static heightfield terrains (Director chapter 15: createTerrain).
    /// Treated as static-only; not integrated, no impulses applied.
    pub terrains: Vec<PhysXTerrain>,

    // ---- Collision callbacks (Director chapter 15) ----
    /// `disableCollision()` global flag.
    pub all_collisions_disabled: bool,
    pub all_callbacks_disabled: bool,
    /// Body-name pairs where collision is filtered. Canonical (min, max) order.
    pub disabled_collision_pairs: std::collections::HashSet<(String, String)>,
    pub disabled_callback_pairs: std::collections::HashSet<(String, String)>,
    /// Single bodies whose collisions are globally disabled.
    pub body_collision_disabled: std::collections::HashSet<String>,
    pub body_callback_disabled: std::collections::HashSet<String>,
    /// `registerCollisionCallback(#handler, scriptRef)`.
    pub collision_callback_handler: Option<String>,
    pub collision_callback_script_ref: Option<crate::player::DatumRef>,
    /// Reports captured during the last Simulate(); drained by NotifyCollisions.
    /// Each entry: (bodyA_id, bodyB_id, contact_points, contact_normals).
    pub pending_collisions: Vec<(u32, u32, Vec<[f64; 3]>, Vec<[f64; 3]>)>,

    // ---- Bookkeeping ----
    pub next_body_id: u32,
    pub next_constraint_id: u32,
    pub next_terrain_id: u32,
    pub last_tick_ms: u64,

    /// When true, the body-vs-body iterative solver routes through the
    /// verbatim PhysX 3.4 SOA path (`PxsSolverSoa` in
    /// `physx_soa_solver.rs`) instead of the in-place AoS sequential-impulse
    /// loop. Body-vs-terrain pairs always run on the AoS path. Default off —
    /// the SoA path is parity-validated in C# but extra wiring (warm-start
    /// cache, synthetic static body for terrains) is still pending.
    pub use_soa_solver: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysXSleepMode { Energy, LinearVelocity }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PhysXTimeStepMode { Equal, Automatic }

impl Default for PhysXPhysicsState {
    fn default() -> Self {
        Self {
            initialized: false, paused: false,
            gravity: [0.0, -9.81, 0.0],
            friction: 0.5, restitution: 0.5,
            linear_damping: 0.0, angular_damping: 0.0,
            contact_tolerance: 0.01,
            sleep_threshold: 0.0316, sleep_mode: PhysXSleepMode::Energy,
            scaling_factor: [1.0, 1.0, 1.0],
            time_step: 1.0 / 60.0,
            time_step_mode: PhysXTimeStepMode::Equal,
            sub_steps: 1,
            sim_time: 0.0,
            three_d_member_name: String::new(),
            bodies: Vec::new(),
            constraints: Vec::new(),
            terrains: Vec::new(),
            all_collisions_disabled: false,
            all_callbacks_disabled: false,
            disabled_collision_pairs: std::collections::HashSet::new(),
            disabled_callback_pairs: std::collections::HashSet::new(),
            body_collision_disabled: std::collections::HashSet::new(),
            body_callback_disabled: std::collections::HashSet::new(),
            collision_callback_handler: None,
            collision_callback_script_ref: None,
            pending_collisions: Vec::new(),
            next_body_id: 1,
            next_constraint_id: 1,
            next_terrain_id: 1,
            last_tick_ms: 0,
            use_soa_solver: false,
        }
    }
}

#[derive(Clone)]
pub struct PhysXPhysicsMember {
    pub state: PhysXPhysicsState,
}

impl fmt::Debug for PhysXPhysicsMember {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(f, "PhysXPhysicsMember(initialized={}, bodies={}, constraints={})",
            self.state.initialized,
            self.state.bodies.len(),
            self.state.constraints.len())
    }
}

impl PhysXPhysicsMember {
    pub fn new() -> Self { Self { state: PhysXPhysicsState::default() } }
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
    HavokPhysics(HavokPhysicsMember),
    PhysXPhysics(PhysXPhysicsMember),
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
    HavokPhysics,
    PhysXPhysics,
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
            Self::HavokPhysics(_) => {
                write!(f, "HavokPhysics")
            }
            Self::PhysXPhysics(_) => {
                write!(f, "PhysXPhysics")
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
            Self::HavokPhysics => Ok("havok"),
            Self::PhysXPhysics => Ok("physics"),
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
            Self::HavokPhysics(_) => CastMemberTypeId::HavokPhysics,
            Self::PhysXPhysics(_) => CastMemberTypeId::PhysXPhysics,
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
            Self::HavokPhysics(_) => "havok",
            Self::PhysXPhysics(_) => "physics",
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

    pub fn as_sound_mut(&mut self) -> Option<&mut SoundMember> {
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

    pub fn as_physx_physics(&self) -> Option<&PhysXPhysicsMember> {
        match self {
            Self::PhysXPhysics(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_physx_physics_mut(&mut self) -> Option<&mut PhysXPhysicsMember> {
        match self {
            Self::PhysXPhysics(data) => Some(data),
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

            // Inline shape sprites (no cast member): cast_lib=0xFFFE, cast_member=0,
            // with non-zero width/height and the sprite-record geometry encoding
            // an oval/rect/etc. directly. Coke Studios' nav_circleanim uses this
            // to draw a radar-ping animation as growing concentric ovals.
            let is_inline_shape = data.cast_lib == 0xFFFE && data.cast_member == 0
                && (data.width > 0 || data.height > 0);

            // Skip empty sprites (no cast member assigned, and not an inline shape).
            // Also skip sprites with cast_lib == 0 which are typically invalid/placeholder entries
            // (cast_lib 65535 is valid - it's used for internal/embedded casts;
            //  cast_lib 65534 is the inline-shape sentinel handled above).
            if (data.cast_member == 0 && !is_inline_shape) || data.cast_lib == 0
                || (data.width == 0 && data.height == 0)
            {
                continue;
            }

            // Director shape sprites — including inline shapes here — center on
            // (pos_x, pos_y). Same offset as bitmaps.
            let (reg_offset_x, reg_offset_y) = (data.width as i32 / 2, data.height as i32 / 2);
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

        if let Some(pfr) = pfr {
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
        debug!(
            "Cast member #{} has unimplemented type: Ole (name: {})",
            number, name
        );
    }

    /// Parse SWF stage dimensions from uncompressed SWF header.
    /// Returns (width, height) in pixels, or None if parsing fails.
    pub fn parse_swf_dimensions(data: &[u8]) -> Option<(u16, u16)> {
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
            reg_point: (0, 0),
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
            fov: 34.516,
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

    fn is_havok_ole(raw: &[u8]) -> bool {
        if raw.len() < 8 { return false; }
        let str_len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if str_len == 0 || str_len > 256 || raw.len() < 4 + str_len { return false; }
        let type_str = std::str::from_utf8(&raw[4..4 + str_len]).ok().unwrap_or("");
        type_str.eq_ignore_ascii_case("havok")
    }

    /// Check if this OLE member is a Havok member by examining both the specific_data_raw
    /// type string and the member info name/file_name fields.
    fn is_havok_member_any(raw: &[u8], member_info: &Option<crate::director::chunks::cast_member_info::CastMemberInfoChunk>) -> bool {
        // Check OLE type string first
        if Self::is_havok_ole(raw) {
            return true;
        }
        // Fall back to checking member_info file_name or exact name "havok"
        if let Some(info) = member_info {
            if info.file_name.to_lowercase().contains("havok") || info.name.eq_ignore_ascii_case("havok") {
                return true;
            }
        }
        false
    }

    /// Detect a PhysX (AGEIA) Physics Xtra OLE member by sniffing the
    /// length-prefixed PROGID string in specific_data_raw. The Director
    /// Physics Xtra registers itself with PROGID "Physics".
    fn is_physx_ole(raw: &[u8]) -> bool {
        if raw.len() < 8 { return false; }
        let str_len = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
        if str_len == 0 || str_len > 256 || raw.len() < 4 + str_len { return false; }
        let type_str = std::str::from_utf8(&raw[4..4 + str_len]).ok().unwrap_or("");
        type_str.eq_ignore_ascii_case("physics")
    }

    /// PhysX detection by either OLE PROGID, file_name hint, or name == "PhysX".
    /// Mirrors `is_havok_member_any`.
    fn is_physx_member_any(raw: &[u8], member_info: &Option<crate::director::chunks::cast_member_info::CastMemberInfoChunk>) -> bool {
        if Self::is_physx_ole(raw) {
            return true;
        }
        if let Some(info) = member_info {
            let fn_lower = info.file_name.to_lowercase();
            if fn_lower.contains("physx") || fn_lower.contains("dynamiks")
                || info.name.eq_ignore_ascii_case("physx")
                || info.name.eq_ignore_ascii_case("physics") {
                return true;
            }
        }
        false
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

        let reg_point = (
            vector_member.reg_point.0 as i32,
            vector_member.reg_point.1 as i32,
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
            reg_point,
        })
    }

    /// Parse the FLSH payload into VectorShapeMember.
    /// Fixed header (160 bytes) + 4 colors (64 bytes) + vertex list.
    /// Parse a FLSH chunk payload into a `VectorShapeMember`. The actual
    /// FLSH byte-layout decoding now lives in
    /// `crate::director::enums::VectorShapeInfo::from(&[u8])`; this thin
    /// wrapper converts the parsed info into a player-side
    /// `VectorShapeMember` and computes the runtime bbox (vertices +
    /// control points + stroke padding) used by the rasterizer.
    fn parse_flsh_payload(data: &[u8]) -> VectorShapeMember {
        VectorShapeMember::from(crate::director::enums::VectorShapeInfo::from(data))
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
        cast_lib: u32,
    ) -> Option<CastMember>
    {
        for opt_child in &member_def.children {
            let Some(Chunk::XMedia(xm)) = opt_child else { continue };

            let member_name = chunk.member_info.as_ref().map(|i| i.name.as_str()).unwrap_or("");
            debug!("Checking XMedia child (member #{}, name='{}', {} bytes)", number, member_name, xm.raw_data.len());

            // 1) If SWF: return SWF
            if let Some(cm) = Self::try_parse_swf(xm.raw_data.to_vec(), number, chunk) {
                debug!("Detected as SWF");
                return Some(cm);
            }

            // 1.5) Check for Havok Physics BEFORE text detection
            // (HKE binary data can be mis-parsed as styled text)
            if Self::is_havok_member_any(&chunk.specific_data_raw, &chunk.member_info) {
                let hke_data = xm.raw_data.clone();
                debug!(
                    "Havok Physics member #{} detected in scan_children_for_ole (early), HKE data={} bytes",
                    number, hke_data.len()
                );
                return Some(CastMember {
                    number,
                    name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                    comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                    member_type: CastMemberType::HavokPhysics(HavokPhysicsMember::new(hke_data)),
                    color: ColorRef::PaletteIndex(255),
                    bg_color: ColorRef::PaletteIndex(0),
                    reg_point: (0, 0),
                });
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
                let hex_dump = xm.raw_data.clone()
                    .iter()
                    .map(|b| format!("{:02X} ", b))
                    .collect::<Vec<String>>()
                    .join(" ");
                debug!(
                    "XMED X2 (text member #{} '{}', {} bytes):\n{}",
                    number,
                    chunk.member_info.as_ref().map(|x| x.name.as_str()).unwrap_or(""),
                    xm.raw_data.len(),
                    hex_dump
                );

                if let Some(styled_text) = xm.parse_styled_text() {
                    debug!("Detected as XMED styled text");
                    let stxt_font_size: Option<u16> = None;
                    return Some(Self::create_text_member_from_xmed(
                        number,
                        chunk,
                        styled_text,
                        stxt_font_size,
                        cast_lib,
                    ));
                }
            }

            // 3) Shockwave3D — IFX IFF container; not a font
            if xm.is_shockwave3d() {
                debug!(
                    "Detected as Shockwave3D (IFX) member #{}, {} bytes",
                    number, xm.raw_data.len()
                );
                let w3d_data = xm.raw_data.clone();
                let parsed_scene = if !w3d_data.is_empty() {
                    match crate::director::chunks::w3d::parse_w3d(&w3d_data) {
                        Ok(mut scene) => {
                            debug!("W3D parsed: {} materials, {} nodes, {} meshes",
                                scene.materials.len(), scene.nodes.len(), scene.clod_meshes.len());
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
                            warn!("W3D parse error: {}", e);
                            Some(std::rc::Rc::new(Self::create_empty_w3d_scene()))
                        }
                    }
                } else {
                    Some(std::rc::Rc::new(Self::create_empty_w3d_scene()))
                };
                let info = Shockwave3dInfo::from(&chunk.specific_data_raw)
                    .unwrap_or(Shockwave3dInfo {
                        loops: false, duration: 0, direct_to_stage: false,
                        animation_enabled: false, preload: false,
                        reg_point: (0, 0), default_rect: (0, 0, 320, 240),
                        camera_position: None, camera_rotation: None,
                        bg_color: None, ambient_color: None,
                    });
                let source_scene = parsed_scene.clone();
                return Some(CastMember {
                    number,
                    name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                    comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                    member_type: { let rs = Shockwave3dRuntimeState::from_info(&info, parsed_scene.as_deref()); CastMemberType::Shockwave3d(Shockwave3dMember { info: info.clone(), w3d_data, source_scene, parsed_scene, runtime_state: rs, converted_from_text: false, text3d_state: None, text3d_source: None }) },
                    color: ColorRef::PaletteIndex(255),
                    bg_color: ColorRef::PaletteIndex(0),
                    reg_point: info.reg_point,
                });
            }

            // 4) Check if this is a real PFR font or just text content
            let has_pfr = Self::extract_pfr(member_def).is_some();
            if has_pfr {
                debug!("Detected as PFR font");
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
            let xmed_script_id = chunk
                .member_info
                .as_ref()
                .map(|info| info.header.script_id)
                .unwrap_or(0);
            if xmed_script_id > 0 {
                text_member.script_id = xmed_script_id;
                text_member.member_script_ref = Some(CastMemberRef {
                    cast_lib: cast_lib as i32,
                    cast_member: xmed_script_id as i32,
                });
            }
            return Some(CastMember {
                number,
                name: member_name,
                comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                member_type: CastMemberType::Text(text_member),
                color: ColorRef::PaletteIndex(255),
                bg_color: ColorRef::PaletteIndex(0),
                reg_point: (0, 0),
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
            reg_point: (0, 0),
        }
    }

    fn create_text_member_from_xmed(
        number: u32,
        chunk: &CastMemberChunk,
        mut styled_text: crate::director::chunks::xmedia::XmedStyledText,
        stxt_font_size: Option<u16>,
        cast_lib: u32,
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

        // Use the FIRST styled span's font face and font size — i.e. the
        // style covering text offset 0. This matches Paige's
        // `pgFindStyleRun(pg, 0, NULL)` lookup that Director uses for
        // `member.fontSize` (and `member.font`) when no selection is
        // active. The C# PgWalkStyle reader confirms it walks Section 6's
        // first character run → its `style_index` → Section 7's font_size.
        //
        // We previously took `max(span_sizes)` to guard against XMED
        // members whose spans store cell-height instead of point size, but
        // that diverged from Director for mixed-style text (e.g. a member
        // with a 24pt heading + 12pt body would report 24 here whereas
        // Director reports whichever style covers offset 0). The STXT
        // correction below still re-anchors `font_size` proportionally
        // when XMED's value disagrees with STXT, so the cell-height case
        // remains protected — we just seed the ratio from the first span
        // instead of the largest.
        //
        // `max` is kept ONLY as a last-ditch fallback when the first span
        // has no font_size at all (rare), so we don't regress to the
        // hardcoded 12 default.
        let (font_name, mut font_size) = if !styled_text.styled_spans.is_empty() {
            let first_style = &styled_text.styled_spans[0].style;
            // Member-level font_size: prefer XMED Section 0x0005 (the
            // par_run_key in Paige's enum, but used as the character-run
            // table by Director) — Director's `the fontSize of member`
            // reads this. Falls back to the first styled span's size, then
            // to max-of-spans, then to 12 (matches the prior behaviour
            // when 0x0005 is absent or empty).
            let first_span_size = first_style
                .font_size
                .filter(|s| *s > 0)
                .map(|s| s as u16);
            let fallback_max = styled_text
                .styled_spans
                .iter()
                .filter_map(|s| s.style.font_size)
                .filter(|s| *s > 0)
                .max()
                .unwrap_or(12) as u16;
            let chosen_size = styled_text
                .default_font_size
                .or(first_span_size)
                .unwrap_or(fallback_max);
            (
                first_style.font_face.clone().unwrap_or_else(|| "Arial".to_string()),
                chosen_size,
            )
        } else {
            ("Arial".to_string(), 12)
        };
        // Correct XMED font sizes using STXT point size when available
        if let Some(stxt_size) = stxt_font_size {
            if stxt_size > 0 && stxt_size != font_size && font_size > 0 {
                let ratio = stxt_size as f32 / font_size as f32;
                for span in &mut styled_text.styled_spans {
                    if let Some(ref mut sz) = span.style.font_size {
                        *sz = ((*sz as f32 * ratio).round() as i32).max(1);
                    }
                }
                font_size = stxt_size;
            }
        }

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
        // Prefer XMED Section 1 dword8C ("fallback height" → `styled_text
        // .height`) for the authored member box height. The `text_info`
        // chunk's height (offset 48-51 of its binary header) tracks a
        // post-layout content extent in many cast versions and grows when
        // multi-line text wraps inside a fixed-size box (CS / FurniFactory
        // clock display: authored 52×18, but `text_info.height = 48` once
        // "Time" + RETURN + value has been laid out — so member.rect was
        // returning 52×48 instead of Director's 52×18).
        // dword8C reflects the authored member dimensions and is stable
        // across content changes, so it's the right source for member.rect.
        let mut box_w = if styled_text.width > 0 {
            styled_text.width as u16
        } else if text_info.width > 0 {
            text_info.width as u16
        } else {
            0
        };
        // For `#fixed` (and other non-adjust) box types, Director reports
        // the AUTHORED single-line height regardless of how many lines of
        // text the member currently holds. The XMED Section 1 dword90
        // ("pageHeight" in the C# parser, Paige's `doc_bottom`) stores the
        // total laid-out height = lines * line_height; dividing by the
        // text's line count yields the stable per-line authored height
        // that matches Director's `member.rect.height`.
        //
        // Verified in PgWalkStyle (C# Paige reader) against the FurniFactory
        // clock display (member 74, "Time"+RETURN+timeValue): page_height=36,
        // line_count=2 → 18, matches Director's `rect(0,0,52,18)`.
        //
        // We compute this BEFORE consulting `text_info.height`, because
        // text_info.height (TextInfo header offset 48-51) is the laid-out
        // content extent — it equals page_height for these members and
        // grows with content. Using it as box_h would balloon a #fixed
        // 52×18 member to 52×48 once two lines of text are written.
        let line_count_for_box = styled_text
            .text
            .replace("\r\n", "\n")
            .replace('\r', "\n")
            .split('\n')
            .filter(|l| !l.is_empty())
            .count()
            .max(1) as u32;
        let box_type_is_adjust = text_info.box_type == 0; // 0=#adjust, 2=#fixed, 3=#limit
        // For non-#adjust members, choose between two interpretations of
        // `text_info.height`:
        //
        //   A. AUTHORED FULL LAYOUT — the value matches the laid-out
        //      `page_height` exactly. Junkbot's level-name member
        //      (#fixed, 15 paragraphs): info_h=page_h=331. Director
        //      reports `member.height = 331` (full authored rect), so we
        //      trust `info_h` verbatim.
        //
        //   B. POST-LAYOUT BALLOON — `info_h > page_h`, the TextInfo
        //      header has grown past the layout extent because content
        //      was added after the member was authored. FurniFactory
        //      clock (#fixed, authored 52×18): info_h=48 once "Time" +
        //      RETURN + value has been laid out, while page_h=36 reflects
        //      the layout. Director reports the AUTHORED 18 here, which
        //      we recover via `page_h / line_count`.
        //
        // The `info_h == page_h` test cleanly separates the two: when
        // they agree, info_h is the authored full layout. When they
        // disagree (info_h > page_h), info_h has ballooned and we divide.
        let per_line_from_page = if !box_type_is_adjust && styled_text.page_height > 0 {
            let info_h = text_info.height;
            let page_h = styled_text.page_height as u32;
            let per_line = if line_count_for_box > 0 {
                page_h / line_count_for_box
            } else {
                0
            };
            // When `info_h` is too small to be multi-line authored
            // (< 2 × per_line) but `page_h` clearly is multi-line
            // (page_h > per_line), prefer page_h. Junkbot V2 names
            // member 6: info_h=24, per_line=24 (page_h=361, 15 lines),
            // 24 % 24 = 0 would trip the multiple-rule and trust 24,
            // but the AUTHORED member.height is 361.
            // Member-height resolution for non-#adjust (boxType=#fixed/
            // #limit/#scroll) text members. Three observed patterns:
            //
            //   • info_h < page_h: page_h is the laid-out total of
            //     current authored content. Use page_h.
            //     Junkbot V2 names (cast 11 member 5): info=348,
            //     page=361 → 361 (Director).
            //
            //   • info_h ≥ page_h with info_h being "real" multi-row
            //     authored:
            //       - info_h == page_h  (member 124: info=page=331)
            //       - info_h is an integer multiple of per_line
            //       - info_h has ≥1 full extra row past page_h
            //         (hint_text member 174: info=107, page=68,
            //          per_line=34, diff=39 ≥ 34 → 107)
            //
            //   • Otherwise (FurniFactory clock balloon: info=48,
            //     page=36, per_line=18 → divide to 18).
            let result = if info_h > 0 && page_h > 0 && per_line > 0 {
                if info_h < page_h {
                    page_h as u16
                } else if info_h == page_h
                    || info_h % per_line == 0
                    || info_h.saturating_sub(page_h) >= per_line
                {
                    info_h as u16
                } else {
                    // info_h ballooned past page_h by less than one row —
                    // a runtime balloon (Lingo wrote text and TextInfo
                    // grew but Paige's doc_bottom = page_h is still the
                    // laid-out total). Use page_h, which matches Director's
                    // reported `member.rect.height` for FurniFactory
                    // displayComputer (member 7) and the clock display
                    // (member 74): both have info_h=48, page_h=36,
                    // per_line=18, line_count=2 and Director returns 36.
                    // Earlier this branch returned `per_line` (18), giving
                    // a single-line height that clipped the second row.
                    page_h as u16
                }
            } else if per_line > 0 {
                per_line as u16
            } else {
                0
            };
            result
        } else {
            0
        };

        // For `#adjust` members whose text has EXPLICIT line breaks
        // (\n / \r), Paige's `doc_bottom` (XMED Section 1 dword90 →
        // `page_height`) is the laid-out total Director reports for
        // member.height. CS Junkbot credits (member 16, 16 \n breaks):
        // page_height=375 matches Director exactly, while text_info.height
        // baked at 483 disagrees. For single-paragraph wrap-only #adjust
        // members (member 82 recycler help: 0 \n, page_height=36, Director
        // dynamically measures wrap to 108) the rule must NOT fire — we
        // fall through to `text_info.height` like before.
        let adjust_use_page_height = box_type_is_adjust
            && styled_text.page_height > 0
            && (styled_text.text.contains('\n') || styled_text.text.contains('\r'));
        let mut box_h = if per_line_from_page > 0 {
            per_line_from_page
        } else if adjust_use_page_height {
            styled_text.page_height as u16
        } else if styled_text.height > 0 {
            styled_text.height as u16
        } else if text_info.height > 0 {
            text_info.height as u16
        } else {
            0
        };

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

        // Final fallback: full page_height (overshoots #fixed but we already
        // tried per-line above; this branch is reached only when both
        // styled_text.height and text_info.height were 0).
        if box_h == 0 && styled_text.page_height > 0 {
            box_h = styled_text.page_height as u16;
        }

        if box_w == 0 { box_w = 100; }
        if box_h == 0 { box_h = 20; }

        // Keep synthesized TextInfo dimensions aligned with effective member box.
        text_info.width = box_w as u32;
        text_info.height = box_h as u32;

        let box_type = text_info.box_type_str().trim_start_matches('#').to_string();
        let word_wrap = text_info.word_wrap();
        let xmed_bg_color = styled_text.bg_color;
        // Member-level fontStyle list: derived from the first styled span's
        // bold/italic/underline flags so `member.fontStyle` matches Director's
        // getter (which returns [#italic] for member 35 etc.). The XMED parse
        // already applies Paige's gap2 -> bool conversion, so we just collect
        // the active span's flags here.
        let member_font_style: Vec<String> = styled_text
            .styled_spans
            .first()
            .map(|s| {
                let mut v = Vec::new();
                if s.style.bold      { v.push("bold".to_string()); }
                if s.style.italic    { v.push("italic".to_string()); }
                if s.style.underline { v.push("underline".to_string()); }
                v
            })
            .unwrap_or_default();
        let xmed_script_id = chunk
            .member_info
            .as_ref()
            .map(|info| info.header.script_id)
            .unwrap_or(0);
        let xmed_member_script_ref = if xmed_script_id > 0 {
            Some(CastMemberRef {
                cast_lib: cast_lib as i32,
                cast_member: xmed_script_id as i32,
            })
        } else {
            None
        };
        let text_member = TextMember {
            text: styled_text.text.clone(),
            html_source: String::new(),
            rtf_source: String::new(),
            alignment: alignment_str.to_string(),
            box_type,
            word_wrap,
            anti_alias: true,
            font: font_name,
            font_style: member_font_style,
            font_size,
            // `styled_text.fixed_line_space` is now derived from par_run[0]'s
            // par_info (the paragraph applied at text position 0) — that's
            // Director's `member.fixedLineSpace` value and is the
            // authoritative source. The older `styled_text.line_spacing`
            // field still exists from the legacy heuristic in
            // `parse_section_8` but is unreliable (lands on 16 for
            // Junkbot v1 level.num where Director reports 21). Use
            // `fixed_line_space` directly; falling through to
            // `styled_text.line_spacing` would re-introduce that bug.
            fixed_line_space: styled_text.fixed_line_space,
            top_spacing: styled_text.top_spacing as i16,
            bottom_spacing: styled_text.bottom_spacing as i16,
            width: box_w,
            height: box_h,
            char_spacing: styled_text.styled_spans.first()
                .map(|s| s.style.char_spacing as i32)
                .unwrap_or(0),
            tab_stops: Vec::new(),
            par_infos: styled_text.par_infos.clone(),
            par_runs: styled_text.par_runs.clone(),
            html_styled_spans: styled_text.styled_spans,
            info: Some(text_info),
            w3d: None,
            sel_start: 0,
            sel_end: 0,
            sel_anchor: 0,
            anti_alias_type: "AutoAlias".to_string(),
            script_id: xmed_script_id,
            member_script_ref: xmed_member_script_ref,
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

        let reg_point = text_member.info.as_ref()
            .map(|info| (info.reg_x, info.reg_y))
            .unwrap_or((0, 0));
        CastMember {
            number,
            name: member_name,
            comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
            member_type: CastMemberType::Text(text_member),
            color: member_color,
            bg_color: member_bg_color,
            reg_point,
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
            CastMemberType::Field(field) => {
                if field.script_id > 0 {
                    Some(field.script_id)
                } else {
                    None
                }
            }
            CastMemberType::Text(text) => {
                if text.script_id > 0 {
                    Some(text.script_id)
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
            CastMemberType::Field(field) => field.member_script_ref.as_ref(),
            CastMemberType::Text(text) => text.member_script_ref.as_ref(),
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
            CastMemberType::Field(field) => {
                field.member_script_ref = Some(script_ref);
            }
            CastMemberType::Text(text) => {
                text.member_script_ref = Some(script_ref);
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

                // Cast-member-attached "BehaviorScript" — Director ships
                // some buttons as Field members with a member script holding
                // mouseDown/mouseUp/mouseUpOutSide. Pulled out of the cast
                // info header the same way ButtonMember does.
                let field_script_id = chunk
                    .member_info
                    .as_ref()
                    .map(|info| info.header.script_id)
                    .unwrap_or(0);
                if field_script_id > 0 {
                    field_member.script_id = field_script_id;
                    field_member.member_script_ref = Some(CastMemberRef {
                        cast_lib: cast_lib as i32,
                        cast_member: field_script_id as i32,
                    });
                }

                debug!(
                    "FieldMember text='{}' alignment='{}' word_wrap={} font='{}' \
                     font_style='{}' font_size={} font_id={:?} fixed_line_space={} \
                     top_spacing={} box_type='{}' anti_alias={} width={} \
                     auto_tab={} editable={} border={} fore_color={:?} back_color={:?} formatting_runs={} script_id={}",
                    field_member.text, field_member.alignment, field_member.word_wrap,
                    field_member.font, field_member.font_style, field_member.font_size,
                    field_member.font_id, field_member.fixed_line_space,
                    field_member.top_spacing, field_member.box_type, field_member.anti_alias,
                    field_member.width, field_member.auto_tab, field_member.editable,
                    field_member.border, field_member.fore_color, field_member.back_color,
                    formatting_runs.len(), field_member.script_id,
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
                        reg_point: (0, 0),
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
                            reg_point: (0, 0),
                        }
                    }
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
                if let Some(cm) = Self::scan_children_for_ole(member_def, number, chunk, bitmap_manager, cast_lib) {
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
                                debug!("W3D parsed: {} materials, {} nodes, {} meshes, {} motions",
                                    scene.materials.len(), scene.nodes.len(), scene.clod_meshes.len(), scene.motions.len());
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
                        info: info.clone(),
                        w3d_data,
                        converted_from_text: false,
                        text3d_state: None,
                        text3d_source: None,
                    }),
                        color: ColorRef::PaletteIndex(255),
                        bg_color: ColorRef::PaletteIndex(0),
                        reg_point: info.reg_point,
                    };
                }

                // Check if this OLE member is a Havok Physics member
                if Self::is_havok_member_any(&chunk.specific_data_raw, &chunk.member_info) {
                    let hke_data = member_def.children.iter()
                        .filter_map(|c| c.as_ref())
                        .find_map(|c| match c {
                            Chunk::XMedia(xm) => Some(xm.raw_data.clone()),
                            Chunk::Raw(raw) if raw.len() > 4 => Some(raw.clone()),
                            _ => None,
                        })
                        .unwrap_or_default();
                    debug!(
                        "Havok Physics member #{} detected, HKE data={} bytes",
                        number, hke_data.len()
                    );
                    return CastMember {
                        number,
                        name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                        comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                        member_type: CastMemberType::HavokPhysics(HavokPhysicsMember::new(hke_data)),
                        color: ColorRef::PaletteIndex(255),
                        bg_color: ColorRef::PaletteIndex(0),
                        reg_point: (0, 0),
                    };
                }

                // Check if this OLE member is a PhysX (AGEIA) Physics member.
                // PhysX members have no authored binary state — the wrapper is
                // empty until Lingo `init()` / `createRigidBody` calls populate it.
                if Self::is_physx_member_any(&chunk.specific_data_raw, &chunk.member_info) {
                    debug!("PhysX (AGEIA) Physics member #{} detected", number);
                    return CastMember {
                        number,
                        name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                        comments: chunk.member_info.as_ref().map(|x| x.comments.to_owned()).unwrap_or_default(),
                        member_type: CastMemberType::PhysXPhysics(PhysXPhysicsMember::new()),
                        color: ColorRef::PaletteIndex(255),
                        bg_color: ColorRef::PaletteIndex(0),
                        reg_point: (0, 0),
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
                if let Some(cm) = Self::scan_children_for_ole(member_def, number, chunk, bitmap_manager, cast_lib) {
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

                debug!("Shape member {} script_id={}", number, script_id);

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

                    let total_frames = {
                        let init_max = score.channel_initialization_data.iter()
                            .map(|(f, _, _)| *f + 1).max().unwrap_or(1);
                        let span_max = score.sprite_spans.iter()
                            .map(|s| s.end_frame).max().unwrap_or(1);
                        let kf_max = score.keyframes_cache.values()
                            .filter_map(|kf| kf.path.as_ref())
                            .flat_map(|p| p.keyframes.iter())
                            .map(|kf| kf.frame).max().unwrap_or(1);
                        init_max.max(span_max).max(kf_max)
                    };
                    CastMemberType::FilmLoop(FilmLoopMember {
                        info: film_loop_info.clone(),
                        score_chunk: score_chunk.clone(),
                        score,
                        current_frame: 1, // Start at frame 1
                        initial_rect,
                        cached_total_frames: Some(total_frames),
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
                        cached_total_frames: None,
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

                // Check if this unknown member is Havok by name or file_name
                if Self::is_havok_member_any(&chunk.specific_data_raw, &chunk.member_info) {
                    let hke_data = member_def.children.iter()
                        .filter_map(|c| c.as_ref())
                        .find_map(|c| match c {
                            Chunk::XMedia(xm) => Some(xm.raw_data.clone()),
                            Chunk::Raw(raw) if raw.len() > 4 => Some(raw.clone()),
                            _ => None,
                        })
                        .unwrap_or_else(|| chunk.specific_data_raw.clone());
                    debug!(
                        "Havok Physics member #{} detected in default branch (type={:?}), HKE data={} bytes",
                        number, chunk.member_type, hke_data.len()
                    );
                    CastMemberType::HavokPhysics(HavokPhysicsMember::new(hke_data))
                } else if Self::is_physx_member_any(&chunk.specific_data_raw, &chunk.member_info) {
                    debug!(
                        "PhysX Physics member #{} detected in default branch (type={:?})",
                        number, chunk.member_type
                    );
                    CastMemberType::PhysXPhysics(PhysXPhysicsMember::new())
                } else {
                    CastMemberType::Unknown
                }
            }
        };

        // Post-processing: if the member wasn't detected as Havok / PhysX by type-specific paths,
        // but its name IS exactly "havok"/"physx" (or file_name contains it), override it.
        // Only override non-Script members to avoid catching scripts like "havokManager".
        let member_type = if !matches!(member_type, CastMemberType::HavokPhysics(_) | CastMemberType::PhysXPhysics(_) | CastMemberType::Script(_)) {
            let name_is_havok = chunk.member_info.as_ref()
                .map(|i| i.name.eq_ignore_ascii_case("havok") || i.file_name.to_lowercase().contains("havok"))
                .unwrap_or(false);
            if name_is_havok {
                let hke_data = member_def.children.iter()
                    .filter_map(|c| c.as_ref())
                    .find_map(|c| match c {
                        Chunk::XMedia(xm) => Some(xm.raw_data.clone()),
                        Chunk::Raw(raw) if raw.len() > 4 => Some(raw.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| chunk.specific_data_raw.clone());
                debug!(
                    "Havok Physics member #{} detected by name override (was {:?}), HKE={} bytes",
                    number, member_type, hke_data.len()
                );
                CastMemberType::HavokPhysics(HavokPhysicsMember::new(hke_data))
            } else {
                // Same override path for PhysX members.
                let name_is_physx = chunk.member_info.as_ref()
                    .map(|i| i.name.eq_ignore_ascii_case("physx")
                        || i.name.eq_ignore_ascii_case("physics")
                        || i.file_name.to_lowercase().contains("physx")
                        || i.file_name.to_lowercase().contains("dynamiks"))
                    .unwrap_or(false);
                if name_is_physx {
                    debug!(
                        "PhysX Physics member #{} detected by name override (was {:?})",
                        number, member_type
                    );
                    CastMemberType::PhysXPhysics(PhysXPhysicsMember::new())
                } else {
                    member_type
                }
            }
        } else {
            member_type
        };

        let reg_point = match &member_type {
            CastMemberType::Bitmap(bm) => (bm.reg_point.0 as i32, bm.reg_point.1 as i32),
            _ => (0, 0),
        };
        // Director surfaces field/button background color through `member.bgColor`.
        // Without propagating field.back_color up to the outer CastMember, the
        // generic getter at cast_member_ref.rs returns the default
        // PaletteIndex(0) for every field — which is also why Coke Studios'
        // `nav_vego_search_field` rendered without a white fill (the field had
        // bgpal RGB white in its STXT, but the renderer's `effective_bg`
        // fallback chain still ended up at the sprite's PaletteIndex(0) on
        // some paths). Mirror Director by surfacing the parsed back_color here.
        let initial_bg_color = match &member_type {
            CastMemberType::Field(fm) => fm
                .back_color
                .clone()
                .unwrap_or(ColorRef::PaletteIndex(0)),
            _ => ColorRef::PaletteIndex(0),
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
            bg_color: initial_bg_color,
            reg_point,
        }
    }
}
