/// PFR1 Font Parser Data Types

use std::collections::HashMap;

// ========== Outline Command Types ==========

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PfrCmdType {
    MoveTo,
    LineTo,
    CurveTo,
    Close,
}

#[derive(Debug, Clone)]
pub struct PfrCmd {
    pub cmd_type: PfrCmdType,
    pub x: f32,
    pub y: f32,
    // Control points for CurveTo (cubic bezier)
    pub x1: f32,
    pub y1: f32,
    pub x2: f32,
    pub y2: f32,
}

impl PfrCmd {
    pub fn move_to(x: f32, y: f32) -> Self {
        Self { cmd_type: PfrCmdType::MoveTo, x, y, x1: 0.0, y1: 0.0, x2: 0.0, y2: 0.0 }
    }
    pub fn line_to(x: f32, y: f32) -> Self {
        Self { cmd_type: PfrCmdType::LineTo, x, y, x1: 0.0, y1: 0.0, x2: 0.0, y2: 0.0 }
    }
    pub fn curve_to(x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) -> Self {
        Self { cmd_type: PfrCmdType::CurveTo, x, y, x1, y1, x2, y2 }
    }
    pub fn close() -> Self {
        Self { cmd_type: PfrCmdType::Close, x: 0.0, y: 0.0, x1: 0.0, y1: 0.0, x2: 0.0, y2: 0.0 }
    }
}

#[derive(Debug, Clone)]
pub struct PfrContour {
    pub commands: Vec<PfrCmd>,
}

impl PfrContour {
    pub fn new() -> Self {
        Self { commands: Vec::new() }
    }
}

// ========== Glyph Types ==========

#[derive(Debug, Clone)]
pub struct OutlineGlyph {
    pub char_code: u32,
    pub set_width: f32,
    pub contours: Vec<PfrContour>,
}

impl OutlineGlyph {
    pub fn new() -> Self {
        Self {
            char_code: 0,
            set_width: 0.0,
            contours: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum PfrStrokeType {
    Line,
    Curve,
    Diagonal,
}

#[derive(Debug, Clone)]
pub struct PfrStroke {
    pub stroke_type: PfrStrokeType,
    pub start_x: f32,
    pub start_y: f32,
    pub end_x: f32,
    pub end_y: f32,
    pub control1_x: f32,
    pub control1_y: f32,
    pub control2_x: f32,
    pub control2_y: f32,
    pub width: f32,
    pub is_horizontal: bool,
    pub is_vertical: bool,
}

impl PfrStroke {
    pub fn line(x1: f32, y1: f32, x2: f32, y2: f32, width: f32) -> Self {
        let is_h = (y2 - y1).abs() < (x2 - x1).abs() * 0.1;
        let is_v = (x2 - x1).abs() < (y2 - y1).abs() * 0.1;
        Self {
            stroke_type: PfrStrokeType::Line,
            start_x: x1, start_y: y1,
            end_x: x2, end_y: y2,
            control1_x: 0.0, control1_y: 0.0,
            control2_x: 0.0, control2_y: 0.0,
            width,
            is_horizontal: is_h,
            is_vertical: is_v,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PfrProgramGlyph {
    pub char_code: u32,
    pub set_width: f32,
    pub strokes: Vec<PfrStroke>,
    pub std_vw: f32,
    pub std_hw: f32,
    pub contours: Vec<PfrContour>,
}

impl PfrProgramGlyph {
    pub fn new() -> Self {
        Self {
            char_code: 0,
            set_width: 0.0,
            strokes: Vec::new(),
            std_vw: 0.0,
            std_hw: 0.0,
            contours: Vec::new(),
        }
    }
}

// ========== Bitmap Glyph ==========

#[derive(Debug, Clone)]
pub struct BitmapGlyph {
    pub char_code: u32,
    pub image_format: u8,
    pub x_pos: i16,
    pub y_pos: i16,
    pub x_size: u16,
    pub y_size: u16,
    pub set_width: u16,
    pub image_data: Vec<u8>,
}

// ========== Font Structure Types ==========

#[derive(Debug, Clone)]
pub struct CharacterRecord {
    pub char_code: u32,
    pub set_width: u16,
    pub gps_size: u32,
    pub gps_offset: u32,
}

#[derive(Debug, Clone)]
pub struct FontMetrics {
    pub units_per_em: u16,
    pub std_vw: i16,
    pub std_hw: i16,
    pub ascender: i16,
    pub descender: i16,
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
    pub flip_x: bool,
    pub flip_y: bool,
}

impl FontMetrics {
    pub fn new() -> Self {
        Self {
            units_per_em: 2048,
            std_vw: 0,
            std_hw: 0,
            ascender: 0,
            descender: 0,
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
            flip_x: false,
            flip_y: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct PhysicalFontRecord {
    pub outline_resolution: u16,
    pub metrics_resolution: u16,
    pub x_min: i16,
    pub y_min: i16,
    pub x_max: i16,
    pub y_max: i16,
    pub flags: u8,
    pub standard_set_width: i16,
    pub char_records: Vec<CharacterRecord>,
    pub font_id: String,
    pub metrics: FontMetrics,
    pub max_x_orus: u8,
    pub max_y_orus: u8,
    // Font-level stroke tables
    pub stroke_x_count: i16,
    pub stroke_y_count: i16,
    pub stroke_x_keys: Vec<i16>,
    pub stroke_y_keys: Vec<i16>,
    pub stroke_x_scales: Vec<i16>,
    pub stroke_y_scales: Vec<i16>,
    pub stroke_x_values: Vec<i32>,
    pub stroke_y_values: Vec<i32>,
    pub stroke_tables_initialized: bool,
    // Blue values (font-wide alignment zones)
    pub blue_values: Vec<i16>,
    pub blue_fuzz: u8,
    pub blue_scale: u8,
    // Extra items
    pub has_bitmap_section: bool,
    pub bitmap_size_table_offset: u32,
    pub gps_offset: u32,
    pub gps_size: u32,
    // Private records from AuxData
    pub private_mode_716: u8,
    pub private_type2_byte28: u8,
    pub private_type2_byte29: u8,
    pub private_flags_492: i32,
    pub private_mode_x: u8,
    pub private_mode_y: u8,
    pub has_extra_item_type5: bool,
    pub extra_type5_word36: i16,
    pub extra_type5_word37: i16,
    pub extra_type5_line_spacing: i16,
    pub extra_type5_word39: i16,
    pub two_byte_char_code: bool,
    // Bitmap strike (extra item type 1, nBitmapSizes>0). Single strike supported.
    pub has_bitmap_strike: bool,
    pub bct_offset: u32,
    pub bct_size: u32,
    pub n_bmap_chars: u32,
    pub bmap_xppm: u16,
    pub bmap_yppm: u16,
    pub bct_three_byte_gps_offset: bool,
    pub bct_two_byte_gps_size: bool,
    pub bct_two_byte_char_code: bool,
}

impl PhysicalFontRecord {
    pub fn new() -> Self {
        Self {
            outline_resolution: 2048,
            metrics_resolution: 2048,
            x_min: 0,
            y_min: 0,
            x_max: 0,
            y_max: 0,
            flags: 0,
            standard_set_width: 0,
            char_records: Vec::new(),
            font_id: String::new(),
            metrics: FontMetrics::new(),
            max_x_orus: 0,
            max_y_orus: 0,
            stroke_x_count: 0,
            stroke_y_count: 0,
            stroke_x_keys: Vec::new(),
            stroke_y_keys: Vec::new(),
            stroke_x_scales: Vec::new(),
            stroke_y_scales: Vec::new(),
            stroke_x_values: Vec::new(),
            stroke_y_values: Vec::new(),
            stroke_tables_initialized: false,
            blue_values: Vec::new(),
            blue_fuzz: 0,
            blue_scale: 0,
            has_bitmap_section: false,
            bitmap_size_table_offset: 0,
            gps_offset: 0,
            gps_size: 0,
            private_mode_716: 4,
            private_type2_byte28: 0,
            private_type2_byte29: 0,
            private_flags_492: 0,
            private_mode_x: 0,
            private_mode_y: 0,
            has_extra_item_type5: false,
            extra_type5_word36: 0,
            extra_type5_word37: 0,
            extra_type5_line_spacing: 0,
            extra_type5_word39: 0,
            two_byte_char_code: false,
            has_bitmap_strike: false,
            bct_offset: 0,
            bct_size: 0,
            n_bmap_chars: 0,
            bmap_xppm: 0,
            bmap_yppm: 0,
            bct_three_byte_gps_offset: false,
            bct_two_byte_gps_size: false,
            bct_two_byte_char_code: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LogicalFontRecord {
    pub font_matrix: [i32; 4],
    pub size: u32,
    pub offset: u32,
    pub style_flags: u16,
}

impl LogicalFontRecord {
    pub fn new() -> Self {
        Self {
            font_matrix: [256, 0, 0, 256],
            size: 0,
            offset: 0,
            style_flags: 0,
        }
    }
}

// ========== PFR Header ==========

#[derive(Debug, Clone)]
pub struct PfrHeader {
    pub version: u16,
    pub signature: u32,
    pub header_sig2: u16,
    pub header_size: u16,
    pub log_font_dir_size: u32,
    pub log_font_dir_offset: u32,
    pub log_font_max_size: u16,
    pub log_font_section_size: u32,
    pub log_font_section_offset: u32,
    pub phys_font_max_size: u16,
    pub phys_font_section_size: u32,
    pub phys_font_section_offset: u32,
    pub gps_max_size: u16,
    pub gps_section_size: u32,
    pub gps_section_offset: u32,
    pub max_blue_values: u8,
    pub max_x_orus: u8,
    pub max_y_orus: u8,
    pub phys_font_max_size_high: u8,
    pub pfr_invert_bitmap: bool,
    pub pfr_black_pixel: bool,
    pub n_phys_fonts: u16,
    pub max_chars: u16,
    pub flags: u8,
}

impl PfrHeader {
    pub fn new() -> Self {
        Self {
            version: 1,
            signature: 0,
            header_sig2: 0,
            header_size: 0,
            log_font_dir_size: 0,
            log_font_dir_offset: 0,
            log_font_max_size: 0,
            log_font_section_size: 0,
            log_font_section_offset: 0,
            phys_font_max_size: 0,
            phys_font_section_size: 0,
            phys_font_section_offset: 0,
            gps_max_size: 0,
            gps_section_size: 0,
            gps_section_offset: 0,
            max_blue_values: 0,
            max_x_orus: 0,
            max_y_orus: 0,
            phys_font_max_size_high: 0,
            pfr_invert_bitmap: false,
            pfr_black_pixel: false,
            n_phys_fonts: 0,
            max_chars: 0,
            flags: 0,
        }
    }
}

// ========== Top-level Parsed Font ==========

#[derive(Debug, Clone)]
pub struct Pfr1ParsedFont {
    pub font_name: String,
    pub header: PfrHeader,
    pub logical_fonts: Vec<LogicalFontRecord>,
    pub physical_font: PhysicalFontRecord,
    pub glyphs: HashMap<u8, OutlineGlyph>,
    pub bitmap_glyphs: HashMap<u8, BitmapGlyph>,
    pub gps_section_offset: u32,
    pub is_pfr1: bool,
    pub pfr_black_pixel: bool,
    pub target_em_px: i32,
    /// True when the glyph outlines are rectilinear (no béziers, axis-aligned
    /// edges) — i.e. a PIXEL/bitmap-style font (Habbo Volter, FFF Reaction…)
    /// rather than a smooth outline font (Verdana, Arial). Computed once from
    /// the parsed glyph geometry (see `classify_pixel_font`). Pixel fonts must
    /// render through the crisp atlas-copy path, NOT the sub-pixel outline
    /// composition (which is for smooth fonts that need anti-aliasing).
    pub is_pixel_font: bool,
}

impl Pfr1ParsedFont {
    pub fn new() -> Self {
        Self {
            font_name: String::new(),
            header: PfrHeader::new(),
            logical_fonts: Vec::new(),
            physical_font: PhysicalFontRecord::new(),
            glyphs: HashMap::new(),
            bitmap_glyphs: HashMap::new(),
            gps_section_offset: 0,
            is_pfr1: true,
            pfr_black_pixel: false,
            target_em_px: 0,
            is_pixel_font: false,
        }
    }

    /// Classify the font as pixel (rectilinear) vs smooth (curvy) from its
    /// glyph geometry, and cache the result in `is_pixel_font`. Pixel fonts are
    /// drawn as axis-aligned rectangles: ~0% bézier curves and ~100% of their
    /// straight edges are horizontal/vertical. Smooth fonts (Verdana/Arial) are
    /// curve-heavy (34–68%); a diagonal display font (Tiki) has few curves but
    /// mostly diagonal edges — both correctly classified as NOT pixel.
    pub fn classify_pixel_font(&mut self) {
        let mut curves: u64 = 0;
        let mut lines: u64 = 0;
        let mut axis_aligned: u64 = 0; // horizontal or vertical LineTo
        for glyph in self.glyphs.values() {
            for contour in &glyph.contours {
                let mut cur = (0.0f32, 0.0f32);
                let mut started = false;
                for cmd in &contour.commands {
                    match cmd.cmd_type {
                        PfrCmdType::MoveTo => { cur = (cmd.x, cmd.y); started = true; }
                        PfrCmdType::LineTo => {
                            if started {
                                let dx = (cmd.x - cur.0).abs();
                                let dy = (cmd.y - cur.1).abs();
                                if dx < 0.5 || dy < 0.5 { axis_aligned += 1; }
                            }
                            lines += 1;
                            cur = (cmd.x, cmd.y);
                        }
                        PfrCmdType::CurveTo => { curves += 1; cur = (cmd.x, cmd.y); }
                        PfrCmdType::Close => {}
                    }
                }
            }
        }
        let total = lines + curves;
        // Need a meaningful sample; tiny/empty fonts default to NOT-pixel
        // (smooth path is the safe fallback — it also handles non-AA via the
        // binary alpha threshold).
        if total < 32 {
            self.is_pixel_font = false;
            return;
        }
        let curve_pct = 100.0 * curves as f64 / total as f64;
        let axis_pct = if lines > 0 { 100.0 * axis_aligned as f64 / lines as f64 } else { 0.0 };
        // Pixel fonts measured at curve≈0 / axis≈100; smooth fonts at
        // curve≥34 / axis≤79. Wide margins on both axes.
        self.is_pixel_font = curve_pct < 5.0 && axis_pct >= 90.0;
    }
}
