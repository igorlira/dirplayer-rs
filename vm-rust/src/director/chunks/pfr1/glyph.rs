/// PFR1 Glyph Parsers
/// Includes: PFR1HeaderParser, PFR1DirectParser, BitmapGlyph parser, scoring

use super::types::*;
use super::log;
use super::stroke_builder;

/// 16.16 fixed-point multiplication.
/// Used by the rasterizer for Director-accurate advance width computation.
pub fn fixed_point_multiply16(a: i32, b: i32) -> i32 {
    let negate = (a < 0) ^ (b < 0);
    let a_abs = a.abs();
    let b_abs = b.abs();
    let hi_a = a_abs >> 16;
    let lo_a = a_abs & 0xFFFF;
    let hi_b = b_abs >> 16;
    let lo_b = b_abs & 0xFFFF;
    let result = (hi_a * b_abs) + ((lo_a * lo_b) >> 16) + (lo_a * hi_b);
    if negate { -result } else { result }
}

// ========== Lookup Tables ==========

/// Curve encoding tables for commands 9, 10, 13
const CURVE_TABLE_9: [u32; 16] = [
    0x0451, 0x0452, 0x0461, 0x0462, 0x0491, 0x0492, 0x04A1, 0x04A2,
    0x0851, 0x0852, 0x0861, 0x0862, 0x0891, 0x0892, 0x08A1, 0x08A2,
];
const CURVE_TABLE_10: [u32; 16] = [
    0x0154, 0x0158, 0x0164, 0x0168, 0x0194, 0x0198, 0x01A4, 0x01A8,
    0x0254, 0x0258, 0x0264, 0x0268, 0x0294, 0x0298, 0x02A4, 0x02A8,
];
const CURVE_TABLE_13: [u32; 16] = [
    0x0FFF, 0x03AA, 0x0CAA, 0x0AA3, 0x0AAC, 0x0AAA, 0x02AA, 0x08AA,
    0x0AA2, 0x0AA8, 0x00AA, 0x0555, 0x0155, 0x0455, 0x0551, 0x0554,
];
const CURVE_TABLE_14A: [u32; 8] = [1, 2, 4, 5, 6, 8, 9, 10];
const CURVE_TABLE_14B: [u32; 4] = [5, 6, 9, 10];

// ========== PFR1 Header Parser ==========

/// PFR1HeaderParser - Primary glyph parser
pub struct Pfr1HeaderParser<'a> {
    data: &'a [u8],
    pos: usize,
    nibble_high: bool,

    // Coordinates
    cur_x: i16,
    cur_y: i16,
    prev_x: i16,
    prev_y: i16,

    // Control coordinate tables
    ctrl_x: Vec<i16>,
    ctrl_y: Vec<i16>,
    scaled_x: Vec<i16>,
    scaled_y: Vec<i16>,
    x_orus_count: usize,
    y_orus_count: usize,

    // Zone transformation tables
    zones_x: Vec<i16>,
    scalars_x: Vec<i16>,
    offsets_x: Vec<i32>,
    zones_y: Vec<i16>,
    scalars_y: Vec<i16>,
    offsets_y: Vec<i32>,
    n_zones_x: i16,
    n_zones_y: i16,

    // Font transform
    font_scale_x: i16,
    font_scale_y: i16,
    font_offset_x: i32,
    font_offset_y: i32,
    coord_shift: i32,
    base_offset: i32,
    orus_zero_scale: f32,
    target_em_px: i32,
    diag_char_code: u32,
    first_point_orus_x: Option<i16>,
    first_point_orus_y: Option<i16>,
    font_metrics: Option<FontMetrics>,
    bbox_x_min: i16,
    bbox_y_min: i16,
    bbox_x_max: i16,
    bbox_y_max: i16,
    scale_counter: i16,
    scale_value_656: i32,
    secondary_scale: i16,
    scaled_pow2: i16,
    scaled_pow2_half: i16,
    scaled_pow2_neg: i16,
    shift_difference: i16,
    final_shift: i16,
    scaled_matrix_a: i16,
    scaled_matrix_b: i16,
    scaled_matrix_c: i16,
    scaled_matrix_d: i16,
    accumulated_2156: i32,
    accumulated_2164: i32,
    rounding_bias_2128: i32,
    boundary_values_532: [i16; 4],
    font_shift_amount: i32,

    // Transform flags
    x_transform_flag: i32,
    y_transform_flag: i32,

    // Font matrix
    matrix_a: i32,
    matrix_b: i32,
    matrix_c: i32,
    matrix_d: i32,

    // Physical font data
    max_x_orus: u8,
    max_y_orus: u8,
    outline_resolution: u16,
    std_vw: f32,
    std_hw: f32,
    // Font-level stroke tables (per PhysicalFontRecord)
    font_stroke_x_count: i16,
    font_stroke_y_count: i16,
    font_stroke_x_keys: Vec<i16>,
    font_stroke_y_keys: Vec<i16>,
    font_stroke_x_scales: Vec<i16>,
    font_stroke_y_scales: Vec<i16>,
    font_stroke_x_values: Vec<i32>,
    font_stroke_y_values: Vec<i32>,
    font_stroke_tables_available: bool,

    // Output
    contours: Vec<PfrContour>,
    current_contour: PfrContour,
    strokes: Vec<PfrStroke>,
    outline_format: u8,
    record_strokes: bool,
    current_stroke_width: f32,
    stroke_has_pos: bool,
    last_stroke_x: f32,
    last_stroke_y: f32,

    // Outline parsing bounds
    outline_end_pos: usize,
    outline_start_pos: usize,

    // GPS context for compound glyph lookup
    // gps_data is the FULL PFR data; gps_base/gps_len define the GPS section within it.
    gps_data: &'a [u8],
    gps_base: i32,
    gps_len: usize,
    glyph_gps_offset: i32,
    known_gps_offsets: Option<&'a [usize]>,

    // Recursion guard for compound glyphs
    recursion_depth: u32,

    // Hint state
    hint_pos: i32,
    hint_nibble_high: i32,
    hint_repeat_count: i32,
    last_interp_x: i16,
    last_interp_y: i16,
    interp_accum_x: i16,
    interp_accum_y: i16,
    flag_620: u32,
    ce9d_nibble_aligned: bool,

    // Recursion depth for compound glyph parsing
    depth: u32,

    // When true, skip compute_coord_shift/compute_font_offsets in parse()
    // (coord state was inherited from parent via copy_parent_coord_state)
    coord_state_inherited: bool,

    // Runtime mode bytes for coordinate path
    cd4_mode_x: u8,
    cd4_mode_y: u8,

    // ===== Grid-fit (hinted) state — active only when `grid_fit` is true =====
    grid_fit: bool,
    // Font-level hint source data (orus), copied from PhysicalFontRecord.
    fit_blue_values: Vec<i16>,
    fit_blue_fuzz: i16,
    fit_blue_ppem: i16,      // byte 124: ppem threshold for overshoot suppression
    fit_stem_snap_v: Vec<i16>, // X-axis stem widths
    fit_stem_snap_h: Vec<i16>, // Y-axis stem widths
    // Per-size fit tables, built once per parse after ComputeFontOffsets.
    fit_tables_ready: bool,
    fit_blue_zones: Vec<[i16; 3]>, // [min, max, value] in scaled sub-pixel units (Y)
    fit_blue_suppress: bool,       // field 2190: size below blue threshold
    fit_word228: i16,              // scaled BlueShift-like limit for top edges
    fit_word230: i16,              // asymmetric rounding bias for top edges
    fit_snap_x: Vec<[i16; 3]>,     // [min, max, value] scaled stem-width snaps (X)
    fit_snap_y: Vec<[i16; 3]>,     // (Y)
    // Per-axis fit mode bytes. Both default to 0 (fitting ON); only render
    // mode 5 takes them from the private record. Bit 4 = plain linear scale
    // (no stem fit), bit 8 = no blue zones.
    fit_mode_x: u8,
    fit_mode_y: u8,
    // Length of the MAIN (header-declared) ctrl tables, before extra-item
    // edges were appended. The outline commands' orus lookups read only these
    // Director resolves them against the raw header-declared tables, not
    // the extras-extended live ones.
    main_ctrl_x_len: usize,
    main_ctrl_y_len: usize,
    // The glyph's set width in OUTLINE units (converted from metrics units),
    // and the resulting re-centering correction:
    // the average (linear - snapped) displacement of the outermost X stem
    // edges inside the advance, in scaled sub-pixel units. Applied along the
    // X transform so a glyph whose stems all snapped one way is pulled back
    // to sit centred in its (linear) advance.
    fit_set_width_orus: i32,
    fit_2192: i32,
}

const MAX_RECURSION_DEPTH: u32 = 10;

impl<'a> Pfr1HeaderParser<'a> {
    pub fn new(
        data: &'a [u8],
        font_matrix: &[i32; 4],
        outline_resolution: u16,
        max_x_orus: u8,
        max_y_orus: u8,
        std_vw: f32,
        std_hw: f32,
        gps_data: &'a [u8],
        gps_base: i32,
        gps_len: usize,
        glyph_gps_offset: i32,
        known_gps_offsets: Option<&'a [usize]>,
        physical_font: Option<&PhysicalFontRecord>,
        font_metrics: Option<&FontMetrics>,
        target_em_px: i32,
    ) -> Self {
        let mut font_stroke_x_count = 0;
        let mut font_stroke_y_count = 0;
        let mut font_stroke_x_keys = Vec::new();
        let mut font_stroke_y_keys = Vec::new();
        let mut font_stroke_x_scales = Vec::new();
        let mut font_stroke_y_scales = Vec::new();
        let mut font_stroke_x_values = Vec::new();
        let mut font_stroke_y_values = Vec::new();
        let mut font_stroke_tables_available = false;
        let mut bbox_x_min: i16 = 0;
        let mut bbox_y_min: i16 = 0;
        let mut bbox_x_max: i16 = outline_resolution as i16;
        let mut bbox_y_max: i16 = outline_resolution as i16;

        if let Some(metrics) = font_metrics {
            bbox_x_min = metrics.x_min;
            bbox_y_min = metrics.y_min;
            bbox_x_max = metrics.x_max;
            bbox_y_max = metrics.y_max;
        }

        if let Some(pf) = physical_font {
            if pf.stroke_tables_initialized {
                font_stroke_x_count = pf.stroke_x_count;
                font_stroke_y_count = pf.stroke_y_count;
                font_stroke_x_keys = pf.stroke_x_keys.clone();
                font_stroke_y_keys = pf.stroke_y_keys.clone();
                font_stroke_x_scales = pf.stroke_x_scales.clone();
                font_stroke_y_scales = pf.stroke_y_scales.clone();
                font_stroke_x_values = pf.stroke_x_values.clone();
                font_stroke_y_values = pf.stroke_y_values.clone();
                font_stroke_tables_available = true;
            }
            if font_metrics.is_none() {
                bbox_x_min = pf.metrics.x_min;
                bbox_y_min = pf.metrics.y_min;
                bbox_x_max = pf.metrics.x_max;
                bbox_y_max = pf.metrics.y_max;
            }
        }

        let mut parser = Self {
            data,
            pos: 0,
            nibble_high: false,
            cur_x: 0,
            cur_y: 0,
            prev_x: 0,
            prev_y: 0,
            ctrl_x: Vec::new(),
            ctrl_y: Vec::new(),
            scaled_x: Vec::new(),
            scaled_y: Vec::new(),
            x_orus_count: 0,
            y_orus_count: 0,
            zones_x: Vec::new(),
            scalars_x: Vec::new(),
            offsets_x: Vec::new(),
            zones_y: Vec::new(),
            scalars_y: Vec::new(),
            offsets_y: Vec::new(),
            n_zones_x: 0,
            n_zones_y: 0,
            font_scale_x: 0,
            font_scale_y: 0,
            font_offset_x: 0,
            font_offset_y: 0,
            coord_shift: 0,
            base_offset: 0,
            orus_zero_scale: 1.0,
            target_em_px,
            diag_char_code: 0,
            font_metrics: font_metrics.cloned(),
            bbox_x_min,
            bbox_y_min,
            bbox_x_max,
            bbox_y_max,
            scale_counter: 0,
            scale_value_656: 0,
            secondary_scale: 0,
            scaled_pow2: 0,
            scaled_pow2_half: 0,
            scaled_pow2_neg: 0,
            shift_difference: 0,
            final_shift: 0,
            scaled_matrix_a: 0,
            scaled_matrix_b: 0,
            scaled_matrix_c: 0,
            scaled_matrix_d: 0,
            accumulated_2156: 0,
            accumulated_2164: 0,
            rounding_bias_2128: 0,
            boundary_values_532: [0; 4],
            font_shift_amount: 0,
            x_transform_flag: 0,
            y_transform_flag: 0,
            matrix_a: font_matrix[0],
            matrix_b: font_matrix[1],
            matrix_c: font_matrix[2],
            matrix_d: font_matrix[3],
            max_x_orus,
            max_y_orus,
            outline_resolution,
            std_vw,
            std_hw,
            font_stroke_x_count,
            font_stroke_y_count,
            font_stroke_x_keys,
            font_stroke_y_keys,
            font_stroke_x_scales,
            font_stroke_y_scales,
            font_stroke_x_values,
            font_stroke_y_values,
            font_stroke_tables_available,
            contours: Vec::new(),
            current_contour: PfrContour::new(),
            strokes: Vec::new(),
            outline_format: 1,
            record_strokes: true,
            current_stroke_width: if std_vw > 0.0 { std_vw } else { 40.0 },
            stroke_has_pos: false,
            last_stroke_x: 0.0,
            last_stroke_y: 0.0,
            outline_end_pos: if data.len() > 0 { data.len() - 1 } else { 0 },
            outline_start_pos: 0,
            gps_data,
            gps_base,
            gps_len,
            glyph_gps_offset,
            known_gps_offsets,
            recursion_depth: 0,
            hint_pos: -1,
            hint_nibble_high: 0,
            hint_repeat_count: 0,
            last_interp_x: 0,
            last_interp_y: 0,
            interp_accum_x: 0,
            interp_accum_y: 0,
            flag_620: 0,
            first_point_orus_x: None,
            first_point_orus_y: None,
            ce9d_nibble_aligned: false,
            depth: 0,
            coord_state_inherited: false,
            cd4_mode_x: 4,
            cd4_mode_y: 4,
            grid_fit: false,
            fit_blue_values: Vec::new(),
            fit_blue_fuzz: 0,
            fit_blue_ppem: 0,
            fit_stem_snap_v: Vec::new(),
            fit_stem_snap_h: Vec::new(),
            fit_tables_ready: false,
            fit_blue_zones: Vec::new(),
            fit_blue_suppress: false,
            fit_word228: 0,
            fit_word230: 0,
            fit_snap_x: Vec::new(),
            fit_snap_y: Vec::new(),
            fit_mode_x: 0,
            fit_mode_y: 0,
            main_ctrl_x_len: usize::MAX,
            main_ctrl_y_len: usize::MAX,
            fit_set_width_orus: 0,
            fit_2192: 0,
        };

        // Initialize cd4 mode from physical font private records
        if let Some(pf) = physical_font {
            if pf.private_mode_716 == 5
                && (pf.private_type2_byte28 != 0 || pf.private_type2_byte29 != 0)
            {
                parser.cd4_mode_x = pf.private_type2_byte28;
                parser.cd4_mode_y = pf.private_type2_byte29;
            } else {
                parser.cd4_mode_x = if pf.private_mode_x == 0 { 4 } else { pf.private_mode_x };
                parser.cd4_mode_y = if pf.private_mode_y == 0 { 4 } else { pf.private_mode_y };
            }
        }

        // Seed fontScaleX/Y from raw matrix absolute values.
        // For targetEmPx > 0, ComputeFontOffsets overrides these with scaled values.
        if target_em_px <= 0 {
            parser.font_scale_x = font_matrix[0].abs() as i16;
            parser.font_scale_y = font_matrix[3].abs() as i16;
        }

        parser
    }

    /// Enable the Director grid-fit path and copy the font-level hint data
    /// (blue values, stem snaps) it feeds on.
    pub fn enable_grid_fit(&mut self, physical_font: Option<&PhysicalFontRecord>) {
        self.grid_fit = true;
        if let Some(pf) = physical_font {
            self.fit_blue_values = pf.blue_values.clone();
            self.fit_blue_fuzz = pf.blue_fuzz as i16;
            self.fit_blue_ppem = pf.blue_scale as i16;
            self.fit_stem_snap_v = pf.stem_snap_v.clone();
            self.fit_stem_snap_h = pf.stem_snap_h.clone();
            // Mode bytes 543/544 default to 0 (fitting on); only private
            // render-mode 5 carries explicit overrides.
            if pf.private_mode_716 == 5 {
                self.fit_mode_x = pf.private_type2_byte28;
                self.fit_mode_y = pf.private_type2_byte29;
            }
        }
    }

    /// Copy grid-fit source data from a parent parser (compound sub-glyphs).
    fn copy_grid_fit_from(&mut self, parent: &Pfr1HeaderParser) {
        self.grid_fit = parent.grid_fit;
        self.fit_blue_values = parent.fit_blue_values.clone();
        self.fit_blue_fuzz = parent.fit_blue_fuzz;
        self.fit_blue_ppem = parent.fit_blue_ppem;
        self.fit_stem_snap_v = parent.fit_stem_snap_v.clone();
        self.fit_stem_snap_h = parent.fit_stem_snap_h.clone();
        self.fit_mode_x = parent.fit_mode_x;
        self.fit_mode_y = parent.fit_mode_y;
    }

    /// Initialize transform flags from font matrix
    fn init_transform_flags(&mut self) {
        let saved_font_scale_x = self.font_scale_x;
        let saved_font_scale_y = self.font_scale_y;
        // Initialize scale and offset to 0
        self.font_scale_x = 0;
        self.font_scale_y = 0;
        self.font_offset_x = 0;
        self.font_offset_y = 0;

        // Default flags = 4 (custom formula)
        self.x_transform_flag = 4;
        self.y_transform_flag = 4;

        // X transformation: check matrixB FIRST
        if self.matrix_b != 0 {
            if self.matrix_a == 0 {
                // fontScaleY = abs(matrixB) — note: Y scale from B!
                self.font_scale_y = self.matrix_b.abs() as i16;
                if self.matrix_b < 0 {
                    self.x_transform_flag = 3;
                } else {
                    self.x_transform_flag = 2;
                }
            }
            // else: flag stays at 4
        } else {
            // matrixB == 0: fontScaleX = abs(matrixA)
            self.font_scale_x = self.matrix_a.abs() as i16;
            if self.matrix_a < 0 {
                self.x_transform_flag = 1;
            } else {
                self.x_transform_flag = 0; // Most common case for standard fonts
            }
        }

        // Y transformation: check matrixC FIRST
        if self.matrix_c != 0 {
            if self.matrix_d == 0 {
                // fontScaleX = abs(matrixC) — note: X scale from C (cross-axis)!
                self.font_scale_x = self.matrix_c.abs() as i16;
                if self.matrix_c < 0 {
                    self.y_transform_flag = 3;
                } else {
                    self.y_transform_flag = 2;
                }
            }
            // else: flag stays at 4
        } else {
            // matrixC == 0: fontScaleY = abs(matrixD)
            self.font_scale_y = self.matrix_d.abs() as i16;
            if self.matrix_d < 0 {
                self.y_transform_flag = 1;
            } else {
                self.y_transform_flag = 0; // Most common case for standard fonts
            }
        }

        // Add baseOffset to fontOffsets
        self.font_offset_x += self.base_offset;
        self.font_offset_y += self.base_offset;

        if let Some(ref metrics) = self.font_metrics {
            if metrics.flip_y {
                self.y_transform_flag = if self.y_transform_flag == 0 || self.y_transform_flag == 2 { 1 } else { 0 };
            }
            if metrics.flip_x {
                self.x_transform_flag = if self.x_transform_flag == 0 || self.x_transform_flag == 2 { 1 } else { 0 };
            }
        }

        if self.font_scale_x == 0 && saved_font_scale_x > 0 {
            self.font_scale_x = saved_font_scale_x;
        }
        if self.font_scale_y == 0 && saved_font_scale_y > 0 {
            self.font_scale_y = saved_font_scale_y;
        }
    }

    /// Compute coordinate shift value
    fn compute_coord_shift(&mut self) {
        let font_matrix16_a = self.matrix_a << 8;
        let font_matrix16_b = self.matrix_b << 8;
        let font_matrix16_c = self.matrix_c << 8;
        let font_matrix16_d = self.matrix_d << 8;

        // The device transform: matrix2136 = FixedPointMultiply16(size << 16,
        // fontMatrix << 8), where `size` is the pixel em being rendered
        // (Director feeds the point size in; the advance formula in the
        // rasterizer uses the same product). The legacy callers pass
        // target_em_px == outline_resolution ("unity"), which keeps
        // coordinates in orus; the grid-fit path passes the real pixel size
        // so the zone tables scale into pixel space.
        let size = if self.target_em_px > 0 {
            self.target_em_px
        } else {
            self.outline_resolution as i32
        };
        let scale_factor = size << 16;

        let matrix2136_a = Self::fixed_point_multiply16(scale_factor, font_matrix16_a);
        let matrix2136_b = Self::fixed_point_multiply16(scale_factor, font_matrix16_b);
        let matrix2136_c = Self::fixed_point_multiply16(scale_factor, font_matrix16_c);
        let matrix2136_d = Self::fixed_point_multiply16(scale_factor, font_matrix16_d);

        let mut max_val = 0;
        for v in [matrix2136_a, matrix2136_b, matrix2136_c, matrix2136_d] {
            let abs_v = v.abs();
            if abs_v > max_val {
                max_val = abs_v;
            }
        }

        let em_size = self.outline_resolution as i16;
        let threshold = (em_size as i32) << 16;

        self.scale_counter = 0;
        let mut v7 = max_val;
        if max_val > threshold {
            loop {
                if self.scale_counter >= 5 {
                    break;
                }
                v7 >>= 2;
                self.scale_counter += 1;
                if v7 <= threshold {
                    break;
                }
            }
        }

        let mut ii = threshold >> 2;
        while max_val <= ii {
            if self.scale_counter <= -4 {
                break;
            }
            self.scale_counter -= 1;
            ii >>= 2;
        }

        self.coord_shift = 13;
        let mut v2 = max_val;
        while v2 >= threshold {
            if self.coord_shift <= 0 {
                break;
            }
            v2 >>= 1;
            self.coord_shift -= 1;
        }

        self.scale_value_656 = (1 << self.coord_shift) >> 1;

        self.scaled_matrix_a = self.scale_matrix_element(matrix2136_a);
        self.scaled_matrix_b = self.scale_matrix_element(matrix2136_b);
        self.scaled_matrix_c = self.scale_matrix_element(matrix2136_c);
        self.scaled_matrix_d = self.scale_matrix_element(matrix2136_d);

        self.accumulated_2156 = 0;
        self.accumulated_2164 = 0;

        let bounds = if self.bbox_x_max != 0 || self.bbox_y_max != 0 {
            [self.bbox_x_min, self.bbox_y_min, self.bbox_x_max, self.bbox_y_max]
        } else {
            [
                0,
                -(self.outline_resolution as i16 / 4),
                self.outline_resolution as i16,
                self.outline_resolution as i16,
            ]
        };

        let (max_norm, v53, v59, v52, v25) = self.compute_max_norm(bounds);

        let v32 = (max_norm >> self.coord_shift) + 3;
        self.secondary_scale = 0;
        let mut j = v32;
        while j <= 0x4000 {
            if self.secondary_scale >= 8 {
                break;
            }
            self.secondary_scale += 1;
            j *= 2;
        }

        self.scaled_pow2 = (1 << self.secondary_scale) as i16;
        self.scaled_pow2_half = (self.scaled_pow2 >> 1) as i16;
        self.scaled_pow2_neg = -self.scaled_pow2;
        self.shift_difference = (self.coord_shift - self.secondary_scale as i32) as i16;
        if self.shift_difference < 0 {
            self.shift_difference = 0;
            self.secondary_scale = self.coord_shift as i16;
        }

        self.final_shift = (16 - self.secondary_scale) as i16;
        self.rounding_bias_2128 = (1 << self.shift_difference) >> 1;
        // base_offset = roundingBias2128
        self.base_offset = self.rounding_bias_2128;

        self.boundary_values_532[0] =
            (((self.rounding_bias_2128 + v53) >> self.shift_difference) - self.scaled_pow2 as i32) as i16;
        self.boundary_values_532[1] =
            (((self.rounding_bias_2128 + v59) >> self.shift_difference) - self.scaled_pow2 as i32) as i16;
        self.boundary_values_532[2] =
            (self.scaled_pow2 as i32 + ((self.rounding_bias_2128 + v52) >> self.shift_difference)) as i16;
        self.boundary_values_532[3] =
            (self.scaled_pow2 as i32 + ((self.rounding_bias_2128 + v25) >> self.shift_difference)) as i16;
    }

    /// Fixed-point division with rounding for matrix elements
    fn scale_matrix_element(&self, raw_matrix: i32) -> i16 {
        let v2 = (self.outline_resolution as i32) << (15 - self.coord_shift);
        let v3 = v2 >> 1;
        let v4 = raw_matrix >> 1;
        if raw_matrix < 0 {
            -((v3 - v4) / v2) as i16
        } else {
            ((v4 + v3) / v2) as i16
        }
    }

    /// 16.16 fixed-point multiplication
    fn fixed_point_multiply16(a: i32, b: i32) -> i32 {
        let negate = (a < 0) ^ (b < 0);
        let a_abs = a.abs();
        let b_abs = b.abs();

        let hi_a = a_abs >> 16;
        let lo_a = a_abs & 0xFFFF;
        let hi_b = b_abs >> 16;
        let lo_b = b_abs & 0xFFFF;

        let result = (hi_a * b_abs) + ((lo_a * lo_b) >> 16) + (lo_a * hi_b);
        if negate { -result } else { result }
    }

    /// Section 8: Compute maximum norm from scaled matrix and bounds
    fn compute_max_norm(&self, bounds: [i16; 4]) -> (i32, i32, i32, i32, i32) {
        let mut max_norm = 0;
        let mut v53 = 0;
        let mut v59 = 0;
        let mut v52 = 0;
        let mut v25_final = 0;

        for pass in 0..2 {
            let v21 = if pass == 0 { self.scaled_matrix_a } else { self.scaled_matrix_b };
            let v15 = if pass == 0 { self.scaled_matrix_c } else { self.scaled_matrix_d };

            let v22 = bounds[0] as i32 * v21 as i32;
            let v23 = bounds[1] as i32 * v15 as i32;
            let mut v25 = v23 + v22;
            let mut v61 = v25;
            let mut v59_local = v25;

            let abs_v25 = v25.abs();
            if abs_v25 > max_norm {
                max_norm = abs_v25;
            }

            for sw in 0..3 {
                let (v27, v28) = match sw {
                    0 => {
                        if v21 == 0 { continue; }
                        ((bounds[2] - bounds[0]) as i32, v21 as i32)
                    }
                    1 => {
                        if v15 == 0 { continue; }
                        ((bounds[3] - bounds[1]) as i32, v15 as i32)
                    }
                    _ => {
                        if v21 == 0 { continue; }
                        ((bounds[0] - bounds[2]) as i32, v21 as i32)
                    }
                };
                v61 += v28 * v27;
                let abs_v61 = v61.abs();
                if abs_v61 > max_norm {
                    max_norm = abs_v61;
                }
                if v61 < v59_local {
                    v59_local = v61;
                }
                if v61 > v25 {
                    v25 = v61;
                }
            }

            if pass == 0 {
                v53 = v59_local;
                v52 = v25;
            } else {
                v59 = v59_local;
                v25_final = v25;
            }
        }

        (max_norm, v53, v59, v52, v25_final)
    }

    /// Compute font offsets (ComputeFontOffsets)
    fn compute_font_offsets(&mut self) {
        // Reset all derived values
        self.font_scale_x = 0;
        self.font_scale_y = 0;
        self.font_offset_x = 0;
        self.font_offset_y = 0;
        self.x_transform_flag = 4;
        self.y_transform_flag = 4;

        // Block 1: X-axis analysis (checks scaled_matrix_c first, then A)
        if self.scaled_matrix_c != 0 {
            if self.scaled_matrix_a == 0 {
                // Rotation/shear: fontScaleY from C, offset into Y
                if self.scaled_matrix_c < 0 {
                    self.font_scale_y = -self.scaled_matrix_c;
                    self.x_transform_flag = 3;
                    self.font_offset_y = -self.accumulated_2156;
                } else {
                    self.font_scale_y = self.scaled_matrix_c;
                    self.x_transform_flag = 2;
                    self.font_offset_y = self.accumulated_2156;
                }
            }
            // Both C and A non-zero: flags stay at 4 (full matrix)
        } else {
            // C == 0 (standard case): fontScaleX from A
            if self.scaled_matrix_a < 0 {
                self.font_scale_x = -self.scaled_matrix_a;
                self.x_transform_flag = 1;
                self.font_offset_x = -self.accumulated_2156;
            } else {
                self.font_scale_x = self.scaled_matrix_a;
                self.x_transform_flag = 0;
                self.font_offset_x = self.accumulated_2156;
            }
        }

        // Block 2: Y-axis analysis (checks scaled_matrix_b first, then D)
        if self.scaled_matrix_b != 0 {
            if self.scaled_matrix_d == 0 {
                // Rotation/shear: fontScaleX from B, offset into X
                if self.scaled_matrix_b < 0 {
                    self.font_scale_x = -self.scaled_matrix_b;
                    self.y_transform_flag = 3;
                    self.font_offset_x = -self.accumulated_2164;
                } else {
                    self.font_scale_x = self.scaled_matrix_b;
                    self.y_transform_flag = 2;
                    self.font_offset_x = self.accumulated_2164;
                }
            }
            // Both B and D non-zero: flags stay at 4 (full matrix)
        } else {
            // B == 0 (standard case): fontScaleY from D
            if self.scaled_matrix_d < 0 {
                self.font_scale_y = -self.scaled_matrix_d;
                self.y_transform_flag = 1;
                self.font_offset_y = -self.accumulated_2164;
            } else {
                self.font_scale_y = self.scaled_matrix_d;
                self.y_transform_flag = 0;
                self.font_offset_y = self.accumulated_2164;
            }
        }

        // Add roundingBias to fontOffset
        self.font_offset_x += self.rounding_bias_2128;
        self.font_offset_y += self.rounding_bias_2128;
        self.font_shift_amount = 16 - self.secondary_scale as i32;

        if self.target_em_px > 0 {
            log(&format!(
                "  [FontOffsets] xFlag={}, yFlag={}, offsetX={}, offsetY={}, bias={}, shiftAmt={}",
                self.x_transform_flag, self.y_transform_flag,
                self.font_offset_x, self.font_offset_y,
                self.rounding_bias_2128, self.font_shift_amount
            ));
        }
    }

    /// Copy CoordShift-derived state from a parent parser.
    /// Sub-glyphs inherit these values and the current matrix state.
    fn copy_parent_coord_state(&mut self, parent: &Pfr1HeaderParser) {
        self.coord_shift = parent.coord_shift;
        self.shift_difference = parent.shift_difference;
        self.secondary_scale = parent.secondary_scale;
        self.scaled_pow2 = parent.scaled_pow2;
        self.scaled_pow2_half = parent.scaled_pow2_half;
        self.scaled_pow2_neg = parent.scaled_pow2_neg;
        self.rounding_bias_2128 = parent.rounding_bias_2128;
        self.base_offset = parent.base_offset;
        self.scale_counter = parent.scale_counter;
        self.scale_value_656 = parent.scale_value_656;
        self.final_shift = parent.final_shift;
        self.boundary_values_532 = parent.boundary_values_532;
        self.scaled_matrix_a = parent.scaled_matrix_a;
        self.scaled_matrix_b = parent.scaled_matrix_b;
        self.scaled_matrix_c = parent.scaled_matrix_c;
        self.scaled_matrix_d = parent.scaled_matrix_d;
        self.accumulated_2156 = parent.accumulated_2156;
        self.accumulated_2164 = parent.accumulated_2164;
        self.coord_state_inherited = true;
    }

    /// Apply component transform (offset + scale) through the matrix,
    /// then re-derive fontScale/flags from modified matrix.
    fn apply_component_transform(&mut self, x_offset: i16, y_offset: i16, x_scale: i32, y_scale: i32) {
        // Offset contribution through matrix
        self.accumulated_2156 += self.scaled_matrix_b as i32 * y_offset as i32
            + self.scaled_matrix_a as i32 * x_offset as i32;
        self.accumulated_2164 += self.scaled_matrix_d as i32 * y_offset as i32
            + self.scaled_matrix_c as i32 * x_offset as i32;
        // Scale matrix by component scale (12-bit fixed-point)
        self.scaled_matrix_a = (((self.scaled_matrix_a as i32) * x_scale + 2048) >> 12) as i16;
        self.scaled_matrix_b = (((self.scaled_matrix_b as i32) * y_scale + 2048) >> 12) as i16;
        self.scaled_matrix_c = (((self.scaled_matrix_c as i32) * x_scale + 2048) >> 12) as i16;
        self.scaled_matrix_d = (((self.scaled_matrix_d as i32) * y_scale + 2048) >> 12) as i16;
        // Re-derive fontScale/flags from modified matrix
        self.compute_font_offsets();
    }

    /// Parse a glyph from GPS data
    pub fn parse(&mut self) -> OutlineGlyph {
        if self.recursion_depth >= MAX_RECURSION_DEPTH {
            return OutlineGlyph::new();
        }

        if !self.coord_state_inherited {
            self.compute_coord_shift();
            self.compute_font_offsets();
        }
        self.strokes.clear();
        self.stroke_has_pos = false;

        if self.data.is_empty() {
            return OutlineGlyph::new();
        }

        // Check if compound glyph BEFORE header parsing
        let outline_format = (self.data[0] >> 6) & 3;
        let is_compound = outline_format >= 2 && (self.data[0] & 0x3F) > 0;

        // Parse glyph header
        let header_result = self.parse_glyph_header();
        if !header_result {
            return OutlineGlyph::new();
        }

        // For compound glyphs, contours were already filled by parse_compound_glyph
        // For simple glyphs, parse outline commands
        if !is_compound {
            self.parse_nibble_commands();
        }

        // Stroke-grid fallback: if no outlines/strokes were produced but we have control grids,
        // parse RLE grid data to synthesize strokes (PFR1 stroke fonts).
        if self.record_strokes
            && self.strokes.is_empty()
            && self.contours.is_empty()
            && !self.ctrl_x.is_empty()
            && !self.ctrl_y.is_empty()
            && self.outline_start_pos < self.data.len()
        {
            self.parse_rle_strokes(self.outline_format as i32, self.outline_start_pos);
        }

        // If stroke glyphs were recorded, convert them to filled contours
        if !self.strokes.is_empty() {
            self.contours = stroke_builder::build_contours_from_strokes(&self.strokes);
        }

        // Post-process (skip for compound glyphs — returns compound contours directly)
        if !is_compound {
            self.remove_duplicate_points();
            // The outlier trim's thresholds are tuned for orus-space coords;
            // in grid-fit pixel space they would eat real points.
            if !self.grid_fit {
                self.trim_contour_outliers();
            }
            self.close_contours();
        }

        let mut glyph = OutlineGlyph::new();
        glyph.contours = self.contours.clone();
        glyph
    }

    /// Parse glyph header
    ///
    /// Bits 0-1 are countEncoding (NOT positionEncoding).
    /// There is NO origin or size parsing for simple glyphs.
    fn parse_glyph_header(&mut self) -> bool {
        if self.pos >= self.data.len() {
            return false;
        }

        let flags = self.data[self.pos];
        self.pos += 1;

        // Extract 2-bit fields
        let outline_format = (flags >> 6) & 3;
        self.outline_format = outline_format;

        // Compound detection: outlineFormat >= 2 AND component count > 0
        // Header byte dual interpretation:
        //   Simple:   bits 6-7=outlineFmt, 4-5=sizeEnc, 2-3=orusEnc, 0-1=posEnc
        //   Compound: bit 7=compound, bit 6=extraData, bits 0-5=componentCount
        let is_compound = outline_format >= 2 && (self.data[0] & 0x3F) > 0;

        if is_compound {
            // For compound glyphs: no origin/orus/size, directly followed by component records
            self.x_orus_count = 0;
            self.y_orus_count = 0;
            self.parse_compound_glyph();
            return true;
        }

        // path for ALL orusEnc values
        // Reset pos to byte 1 (we already read byte 0 as flags)
        self.pos = 1;

        // interprets bits 0-1 as count encoding mode
        let count_encoding = flags & 3;
        // all orusEnc values still use nibble parser and contour rendering.
        // Disable stroke recording during outline parsing.
        self.record_strokes = false;
        self.strokes.clear();
        self.stroke_has_pos = false;

        self.x_orus_count = 0;
        self.y_orus_count = 0;

        match count_encoding {
            0 => {
                // No control points
            }
            1 => {
                // Control counts from byte 1 nibbles
                // X count is LOW nibble, Y count is HIGH nibble
                if self.pos < self.data.len() {
                    let count_byte = self.data[self.pos];
                    self.pos += 1;
                    self.x_orus_count = (count_byte & 0x0F) as usize;        // Low nibble = X count
                    self.y_orus_count = ((count_byte >> 4) & 0x0F) as usize;  // High nibble = Y count
                }
            }
            2 | 3 => {
                // Control counts from separate bytes
                if self.pos + 1 < self.data.len() {
                    self.x_orus_count = self.data[self.pos] as usize;
                    self.pos += 1;
                    self.y_orus_count = self.data[self.pos] as usize;
                    self.pos += 1;
                }
            }
            _ => {}
        }

        // Parse control values if any
        if self.x_orus_count > 0 || self.y_orus_count > 0 {
            self.parse_ce9d_control_values(flags);
        } else {
            self.ctrl_x.clear();
            self.ctrl_y.clear();
            self.ce9d_nibble_aligned = false;
        }
        // Snapshot the main table lengths before extras can extend them —
        // the outline commands' orus lookups see only these entries.
        self.main_ctrl_x_len = self.ctrl_x.len();
        self.main_ctrl_y_len = self.ctrl_y.len();

        // Skip extra items if flag bit 3 is set
        // Extra items are after control values but BEFORE outline commands.
        // The grid-fit path processes them (extra controlled edges); the
        // legacy path only skips them.
        let has_extra_items = (flags & 0x08) != 0;
        let extra_items_start = self.pos;
        if has_extra_items && self.pos < self.data.len() {
            let extra_count = self.data[self.pos] as usize;
            self.pos += 1;
            for _ in 0..extra_count {
                if self.pos + 1 >= self.data.len() { break; }
                let item_len = self.data[self.pos] as usize;
                self.pos += item_len + 2; // Skip length + type + data
            }
        }
        let extra_items_range = if has_extra_items {
            Some((extra_items_start, self.pos.min(self.data.len())))
        } else {
            None
        };

        // Record outline start position (after extra items / control values)
        self.outline_start_pos = self.pos;

        // Set flag620 for fontScale handling
        self.flag_620 = 0;
        if self.font_scale_x == 0 {
            self.flag_620 |= 1;
            self.font_scale_x = 256;
        }
        if self.font_scale_y == 0 {
            self.flag_620 |= 2;
            self.font_scale_y = 256;
        }

        // Compute scaled coordinates and zone tables
        if self.grid_fit {
            self.fit_scaled_and_zones(extra_items_range);
        } else {
            self.compute_scaled_coordinates();
            self.initialize_zone_tables();
        }

        // uses exact glyph size - 1 as outline end position
        self.outline_end_pos = if self.data.len() > 0 { self.data.len() - 1 } else { 0 };

        // Scale output coordinates from secondary_scale space back to orus space.
        // The coordinate pipeline (compute_coord_shift + apply_transform_flags) right-shifts
        // by secondary_scale, so we multiply by 2^secondary_scale to recover orus coordinates.
        // Grid-fit: keep the sub-pixel fraction. apply_transform_flags skips
        // its truncating `>> secondary_scale`, and the f32 conversion divides
        // by the pixel unit instead.
        self.orus_zero_scale = if self.grid_fit && self.scaled_pow2 > 0 {
            1.0 / self.scaled_pow2 as f32
        } else {
            1.0
        };

        // Initialize hint stream state (reads backwards from end of glyph data)
        self.hint_pos = self.outline_end_pos as i32;
        self.hint_nibble_high = 1;
        self.hint_repeat_count = 0;
        self.last_interp_x = 0;
        self.last_interp_y = 0;
        self.interp_accum_x = 0;
        self.interp_accum_y = 0;

        true
    }

    /// Parse CE9D-style control coordinate values
    /// Uses accumulative deltas with flag caching, NOT simple nibble reads
    fn parse_ce9d_control_values(&mut self, flags: u8) {
        self.ctrl_x.clear();
        self.ctrl_y.clear();

        if self.x_orus_count == 0 && self.y_orus_count == 0 {
            return;
        }

        let three_byte_mode = (flags & 3) == 3;
        let flag_per_coord = (flags & 0x40) != 0;
        let x_enc_mode = ((flags >> 4) & 1) as i32;
        let y_enc_mode = ((flags >> 5) & 1) as i32;

        // nibble alignment starts at false
        let mut nibble_aligned = false;
        // Flag nibble cache
        let mut flag_cache: i32 = 0;
        let mut flag_cache_count: i32 = 0;

        // Parse X control values with accumulative deltas
        let mut accum_x: i16 = 0;
        for i in 0..self.x_orus_count {
            if self.pos >= self.data.len() { break; }

            let v8;
            if i == 0 {
                v8 = x_enc_mode; // First X uses xEncMode from flags
            } else if flag_per_coord {
                let result = self.read_ce9d_flag_nibble_cached(nibble_aligned, flag_cache, flag_cache_count);
                v8 = result.0;
                nibble_aligned = result.1;
                flag_cache = result.2;
                flag_cache_count = result.3;
            } else {
                v8 = 0; // No per-coord flags, use single-byte mode
            }

            let result = self.read_ce9d_coord_value_new(v8, three_byte_mode, nibble_aligned);
            let delta = result.0;
            nibble_aligned = result.1;
            accum_x = accum_x.wrapping_add(delta);
            self.ctrl_x.push(accum_x);
        }

        // Parse Y control values with accumulative deltas
        let mut accum_y: i16 = 0;
        for i in 0..self.y_orus_count {
            if self.pos >= self.data.len() { break; }

            let v8;
            if i == 0 {
                v8 = y_enc_mode; // First Y uses yEncMode from flags
            } else if flag_per_coord {
                let result = self.read_ce9d_flag_nibble_cached(nibble_aligned, flag_cache, flag_cache_count);
                v8 = result.0;
                nibble_aligned = result.1;
                flag_cache = result.2;
                flag_cache_count = result.3;
            } else {
                v8 = 0;
            }

            let result = self.read_ce9d_coord_value_new(v8, three_byte_mode, nibble_aligned);
            let delta = result.0;
            nibble_aligned = result.1;
            accum_y = accum_y.wrapping_add(delta);
            self.ctrl_y.push(accum_y);
        }

        // POST-PROCESSING
        // If bit 2 of flags is set, insert first Y value at beginning
        if (flags & 4) != 0 && !self.ctrl_y.is_empty() {
            let first_y = self.ctrl_y[0];
            self.ctrl_y.insert(0, first_y);
        }

        // If odd Y count, duplicate last value
        if (self.ctrl_y.len() & 1) != 0 && !self.ctrl_y.is_empty() {
            let last_y = *self.ctrl_y.last().unwrap();
            self.ctrl_y.push(last_y);
        }

        // Byte-align at exit
        if nibble_aligned {
            self.pos += 1;
        }
        self.ce9d_nibble_aligned = false;
    }

    /// Read flag nibble with caching
    fn read_ce9d_flag_nibble_cached(&mut self, mut aligned: bool, mut cache: i32, mut count: i32) -> (i32, bool, i32, i32) {
        if count > 0 {
            // Use cached flag bits (shift right by 1 each time)
            let v13 = (cache >> 1) & 0x7F;
            return (v13 & 1, aligned, v13, count - 1);
        }

        // Read new flag nibble
        if self.pos >= self.data.len() {
            return (0, aligned, 0, 0);
        }

        let v12 = self.data[self.pos];
        let v13;
        if aligned {
            // Low nibble, advance
            v13 = (v12 & 0x0F) as i32;
            self.pos += 1;
            aligned = false;
        } else {
            // High nibble, don't advance
            v13 = (v12 >> 4) as i32;
            aligned = true;
        }
        (v13 & 1, aligned, v13, 3) // 3 more uses from this nibble
    }

    /// Read a coordinate value
    fn read_ce9d_coord_value_new(&mut self, v8: i32, three_byte_mode: bool, mut nibble_aligned: bool) -> (i16, bool) {
        if self.pos >= self.data.len() {
            return (0, nibble_aligned);
        }

        let multiplier: i32 = 16; // always 16

        if (v8 & 1) == 0 {
            // Single-byte mode
            let result;
            if nibble_aligned {
                // Read: low nibble of current byte + high nibble of next byte
                let low_nibble = (self.data[self.pos] & 0x0F) as i32;
                self.pos += 1;
                if self.pos >= self.data.len() {
                    return (((low_nibble << 4) as i16), true);
                }
                let high_nibble = (self.data[self.pos] >> 4) as i32;
                result = ((low_nibble << 4) | high_nibble) as i16;
                // Still nibble-aligned (consumed high nibble of new byte)
            } else {
                // Full byte read
                result = self.data[self.pos] as i16;
                self.pos += 1;
            }
            (result, nibble_aligned)
        } else {
            // 1.5-byte mode
            if three_byte_mode {
                // 3-byte encoding mode - full 16-bit values
                let result;
                if nibble_aligned {
                    let b0_low = if self.pos > 0 { (self.data[self.pos - 1] & 0x0F) as i32 } else { 0 };
                    let b1 = self.data[self.pos] as i32;
                    self.pos += 1;
                    let b2_high = if self.pos < self.data.len() { (self.data[self.pos] >> 4) as i32 } else { 0 };
                    result = ((b0_low << 12) | (b1 << 4) | b2_high) as i16;
                } else {
                    let b0 = self.data[self.pos] as i32;
                    self.pos += 1;
                    let b1 = if self.pos < self.data.len() { self.data[self.pos] as i32 } else { 0 };
                    if self.pos < self.data.len() { self.pos += 1; }
                    result = ((b0 << 8) | b1) as i16;
                }
                (result, nibble_aligned)
            } else {
                // Standard 1.5-byte mode
                let result;
                if nibble_aligned {
                    // low nibble + full next byte
                    let low_nibble = (self.data[self.pos] & 0x0F) as i32;
                    self.pos += 1;
                    let next_byte = if self.pos < self.data.len() {
                        let b = self.data[self.pos] as i32;
                        self.pos += 1;
                        b
                    } else { 0 };
                    // Sign-extend nibble: nibble >= 8 is negative
                    let shifted_nibble = ((low_nibble << 4) as i8) as i32;
                    result = (next_byte + multiplier * shifted_nibble) as i16;
                    nibble_aligned = false; // Now byte-aligned
                } else {
                    // signed byte + high nibble
                    let signed_byte = self.data[self.pos] as i8 as i32;
                    self.pos += 1;
                    let next_high_nibble = if self.pos < self.data.len() {
                        (self.data[self.pos] >> 4) as i32
                    } else { 0 };
                    result = (next_high_nibble + multiplier * signed_byte) as i16;
                    nibble_aligned = true;
                }
                (result, nibble_aligned)
            }
        }
    }

    /// Low-level nibble reader (raw, toggle-first protocol for nibble commands)
    fn read_nibble_raw(&mut self) -> i32 {
        if self.pos >= self.data.len() { return -1; }

        // Toggle first, then read
        self.nibble_high = !self.nibble_high;

        if self.nibble_high {
            // Read high nibble, no advance
            (self.data[self.pos] >> 4) as i32
        } else {
            // Read low nibble, advance
            let val = (self.data[self.pos] & 0x0F) as i32;
            self.pos += 1;
            val
        }
    }

    /// Read a nibble using toggle-first protocol
    fn read_nibble(&mut self) -> i32 {
        if self.pos >= self.data.len() { return -1; }

        // Toggle first, then read
        self.nibble_high = !self.nibble_high;

        if self.nibble_high {
            (self.data[self.pos] >> 4) as i32
        } else {
            let val = (self.data[self.pos] & 0x0F) as i32;
            self.pos += 1;
            val
        }
    }

    /// Read a byte-aligned value from nibble stream
    fn read_byte_aligned(&mut self) -> i32 {
        if self.pos >= self.data.len() { return 0; }

        if self.nibble_high {
            // Cross-byte read: low nibble of current + high nibble of next
            let low_nibble = (self.data[self.pos] & 0x0F) as i32;
            self.pos += 1;
            if self.pos >= self.data.len() {
                return low_nibble << 4;
            }
            let high_nibble = (self.data[self.pos] >> 4) as i32;
            (low_nibble << 4) | high_nibble
        } else {
            // Byte-aligned read
            let val = self.data[self.pos] as i32;
            self.pos += 1;
            val
        }
    }

    /// Parse nibble-based outline commands
    fn parse_nibble_commands(&mut self) {
        self.cur_x = 0;
        self.cur_y = 0;
        self.prev_x = 0;
        self.prev_y = 0;
        self.nibble_high = false;

        let mut iterations = 0;
        let max_iterations = self.data.len() * 2; // at most 2 commands per byte
        let mut first_iteration = true;

        while iterations < max_iterations {
            iterations += 1;

            // Termination check
            if self.pos >= self.outline_end_pos && (self.pos != self.outline_end_pos || self.nibble_high) {
                break;
            }
            if self.pos >= self.data.len() {
                break;
            }

            let cmd;
            if first_iteration {
                // First call forces cmd=6 (MoveTo)
                cmd = 6;
                first_iteration = false;
            } else {
                cmd = self.read_nibble();
                if cmd < 0 { break; }
            }

            self.process_command(cmd);
        }

        self.finish_contour();
    }

    /// Parse RLE-encoded stroke grid and generate strokes (PFR1 stroke fonts)
    /// GenerateStrokesFromGrid + RLE parsing logic.
    fn parse_rle_strokes(&mut self, format_bits: i32, start_pos: usize) {
        let rows = self.ctrl_y.len();
        let cols = self.ctrl_x.len();
        if rows == 0 || cols == 0 {
            return;
        }

        let total_cells = rows * cols;
        let mut drawn_cells = vec![false; total_cells];

        let mut cell_index: usize = 0;
        let mut rle_pos = start_pos;

        while cell_index < total_cells && rle_pos < self.data.len() {
            let mut skip = 0usize;
            let mut draw = 0usize;

            if format_bits == 1 {
                // Nibble-based RLE: accumulate until stop condition
                let mut remaining = total_cells.saturating_sub(cell_index);
                while remaining > 0 && rle_pos < self.data.len() {
                    let b = self.data[rle_pos];
                    let high = ((b >> 4) & 0x0F) as usize;
                    let low = (b & 0x0F) as usize;

                    skip += high;
                    draw += low;
                    remaining = remaining.saturating_sub(high + low);
                    rle_pos += 1;

                    // Stop: low != 0 AND next_byte != 0
                    if low != 0 && rle_pos < self.data.len() && self.data[rle_pos] != 0 {
                        break;
                    }
                }
            } else if format_bits == 2 {
                // Byte-based RLE
                if rle_pos + 1 >= self.data.len() {
                    break;
                }
                skip = self.data[rle_pos] as usize;
                draw = self.data[rle_pos + 1] as usize;
                rle_pos += 2;
            } else {
                // Bit-based (format 0) not implemented: mark all cells as drawn
                for v in drawn_cells.iter_mut() {
                    *v = true;
                }
                break;
            }

            // Skip cells
            cell_index = cell_index.saturating_add(skip);

            // Mark drawn cells using column-major order with inverted Y
            for _ in 0..draw {
                if cell_index >= total_cells {
                    break;
                }
                let col = cell_index / rows;
                let raw_row = cell_index % rows;
                let row = rows - 1 - raw_row; // invert Y
                if row < rows && col < cols {
                    drawn_cells[row * cols + col] = true;
                }
                cell_index += 1;
            }
        }

        self.generate_strokes_from_grid(&drawn_cells, rows, cols, self.current_stroke_width);
    }

    /// Generate strokes from a grid of drawn cells.
    fn generate_strokes_from_grid(
        &mut self,
        drawn_cells: &[bool],
        rows: usize,
        cols: usize,
        stroke_width: f32,
    ) {
        if rows == 0 || cols == 0 {
            return;
        }

        // Grid-fit: stroke endpoints come from the FITTED scaled tables (in
        // pixel units), not the raw orus ctrl values — otherwise stroke-grid
        // glyphs leak orus-space coordinates into a pixel-space glyph.
        let (ctrl_x_px, ctrl_y_px): (Vec<f32>, Vec<f32>) = if self.grid_fit && self.scaled_pow2 > 0 {
            let inv = 1.0 / self.scaled_pow2 as f32;
            (
                self.scaled_x.iter().map(|&v| v as f32 * inv).collect(),
                self.scaled_y.iter().map(|&v| v as f32 * inv).collect(),
            )
        } else {
            (
                self.ctrl_x.iter().map(|&v| v as f32).collect(),
                self.ctrl_y.iter().map(|&v| v as f32).collect(),
            )
        };
        let stroke_width = if self.grid_fit && self.scaled_pow2 > 0 && self.outline_resolution > 0 {
            // Stroke width is in orus; convert to pixels for grid-fit space.
            (stroke_width * self.target_em_px as f32 / self.outline_resolution as f32).max(1.0)
        } else {
            stroke_width
        };

        // Horizontal runs
        for r in 0..rows {
            let mut start_col: isize = -1;
            for c in 0..=cols {
                let drawn = c < cols && drawn_cells[r * cols + c];
                if drawn && start_col < 0 {
                    start_col = c as isize;
                } else if !drawn && start_col >= 0 {
                    let y = ctrl_y_px[r];
                    let x1 = ctrl_x_px[start_col as usize];
                    let x2 = ctrl_x_px[(c.saturating_sub(1)).min(cols - 1)];

                    if (x2 - x1).abs() > 1.0 {
                        self.strokes.push(PfrStroke::line(x1, y, x2, y, stroke_width));
                    }
                    start_col = -1;
                }
            }
        }

        // Vertical runs
        for c in 0..cols {
            let mut start_row: isize = -1;
            for r in 0..=rows {
                let drawn = r < rows && drawn_cells[r * cols + c];
                if drawn && start_row < 0 {
                    start_row = r as isize;
                } else if !drawn && start_row >= 0 {
                    let x = ctrl_x_px[c];
                    let y1 = ctrl_y_px[start_row as usize];
                    let y2 = ctrl_y_px[(r.saturating_sub(1)).min(rows - 1)];

                    if (y2 - y1).abs() > 1.0 {
                        self.strokes.push(PfrStroke::line(x, y1, x, y2, stroke_width));
                    }
                    start_row = -1;
                }
            }
        }

        // Fallback: boundary rectangle if drawn cells exist but no strokes
        if self.strokes.is_empty() && drawn_cells.iter().any(|v| *v) {
            let fold_min = |v: &[f32]| v.iter().cloned().fold(f32::MAX, f32::min);
            let fold_max = |v: &[f32]| v.iter().cloned().fold(f32::MIN, f32::max);
            let min_x = if ctrl_x_px.is_empty() { 0.0 } else { fold_min(&ctrl_x_px) };
            let max_x = if ctrl_x_px.is_empty() { 0.0 } else { fold_max(&ctrl_x_px) };
            let min_y = if ctrl_y_px.is_empty() { 0.0 } else { fold_min(&ctrl_y_px) };
            let max_y = if ctrl_y_px.is_empty() { 0.0 } else { fold_max(&ctrl_y_px) };

            self.strokes.push(PfrStroke::line(min_x, min_y, min_x, max_y, stroke_width));
            self.strokes.push(PfrStroke::line(max_x, min_y, max_x, max_y, stroke_width));
            self.strokes.push(PfrStroke::line(min_x, min_y, max_x, min_y, stroke_width));
            self.strokes.push(PfrStroke::line(min_x, max_y, max_x, max_y, stroke_width));
        }
    }

    /// Process a single nibble command
    fn process_command(&mut self, cmd: i32) {
        match cmd {
            0 => self.process_small_delta(),
            1 => self.process_x_byte_delta(),
            2 => self.process_y_byte_delta(),
            3 => self.process_x_word_delta(),
            4 => self.process_y_word_delta(),
            5 => self.process_line_encoded(true),
            6 => self.process_line_encoded(false),
            7..=15 => self.process_curve(cmd),
            _ => {}
        }
    }

    /// Process command 0 (small delta)
    fn process_small_delta(&mut self) {
        let nibble = self.read_nibble();
        if nibble < 0 { return; }

        let mut v44 = self.cur_x;
        let mut v45 = self.cur_y;

        let direction: i16 = if (nibble & 4) != 0 {
            (nibble & 7) as i16 - 8
        } else {
            (nibble & 7) as i16 + 1
        };

        if (nibble & 8) != 0 {
            v45 = self.orus_lookup(1, direction);
        } else {
            v44 = self.orus_lookup(0, direction);
        }

        self.update_coordinates_and_add_point(v44, v45);
    }

    /// Process command 1 (X byte delta)
    fn process_x_byte_delta(&mut self) {
        let v45 = self.cur_y;
        let delta = self.read_byte_aligned();
        let signed_delta = delta as i8;
        let v44 = self.cur_x.wrapping_add(signed_delta as i16);
        self.update_coordinates_and_add_point(v44, v45);
    }

    /// Process command 2 (Y byte delta)
    fn process_y_byte_delta(&mut self) {
        let v44 = self.cur_x;
        let delta = self.read_byte_aligned();
        let signed_delta = delta as i8;
        let v45 = self.cur_y.wrapping_add(signed_delta as i16);
        self.update_coordinates_and_add_point(v44, v45);
    }

    /// Process command 3 (X word delta)
    fn process_x_word_delta(&mut self) {
        let v45 = self.cur_y;
        let hi_byte = self.read_byte_aligned();
        let hi_signed = hi_byte as i8;

        // Toggle after byte read, before nibble read
        self.nibble_high = !self.nibble_high;

        let lo = if self.pos >= self.data.len() {
            0
        } else if self.nibble_high {
            (self.data[self.pos] >> 4) as i32
        } else {
            let v = (self.data[self.pos] & 0x0F) as i32;
            self.pos += 1;
            v
        };

        let delta12 = ((hi_signed as i32) << 4) | lo;

        let v44 = if delta12 >= -128 && delta12 < 128 {
            let extra = self.read_inline_extra_byte();
            let delta16 = (delta12 << 8) | extra;
            self.cur_x.wrapping_add(delta16 as i16)
        } else {
            self.cur_x.wrapping_add(delta12 as i16)
        };

        self.update_coordinates_and_add_point(v44, v45);
    }

    /// Process command 4 (Y word delta)
    fn process_y_word_delta(&mut self) {
        let v44 = self.cur_x;
        let hi_byte = self.read_byte_aligned();
        let hi_signed = hi_byte as i8;

        self.nibble_high = !self.nibble_high;

        let lo = if self.pos >= self.data.len() {
            0
        } else if self.nibble_high {
            (self.data[self.pos] >> 4) as i32
        } else {
            let v = (self.data[self.pos] & 0x0F) as i32;
            self.pos += 1;
            v
        };

        let delta12 = ((hi_signed as i32) << 4) | lo;

        let v45 = if delta12 >= -128 && delta12 < 128 {
            let extra = self.read_inline_extra_byte();
            let delta16 = (delta12 << 8) | extra;
            self.cur_y.wrapping_add(delta16 as i16)
        } else {
            self.cur_y.wrapping_add(delta12 as i16)
        };

        self.update_coordinates_and_add_point(v44, v45);
    }

    /// Read inline extra byte for word delta commands
    fn read_inline_extra_byte(&mut self) -> i32 {
        if self.pos >= self.data.len() {
            return 0;
        }
        if self.nibble_high {
            let lo_nib = (self.data[self.pos] & 0x0F) as i32;
            self.pos += 1;
            let hi_nib = if self.pos < self.data.len() {
                (self.data[self.pos] >> 4) as i32
            } else {
                0
            };
            (lo_nib << 4) | hi_nib
        } else {
            let v = self.data[self.pos] as i32;
            self.pos += 1;
            v
        }
    }

    /// Process commands 5-6 (line with encoding)
    fn process_line_encoded(&mut self, add_point: bool) {
        let enc = self.read_nibble();
        if enc < 0 { return; }

        let mut v44 = self.cur_x;
        let mut v45 = self.cur_y;

        self.read_encoded_coord_pair_into(enc, &mut v44, &mut v45);

        if add_point {
            self.add_point_only();
        } else {
            // Command 6: close current contour, start new
            if !self.current_contour.commands.is_empty() {
                self.finish_contour();
            }

            let (out_x, out_y) = self.transform_point(self.cur_x, self.cur_y);
            let (final_x, final_y) = self.apply_transform_flags_with_raw(self.cur_x, self.cur_y, out_x, out_y);
            let scaled_x = final_x as f32 * self.orus_zero_scale;
            let scaled_y = final_y as f32 * self.orus_zero_scale;
            self.current_contour.commands.push(PfrCmd::move_to(scaled_x, scaled_y));
        }
    }

    /// Process curve commands 7-15
    fn process_curve(&mut self, cmd: i32) {
        let mut v5: u32 = 0;
        let mut path: u32 = 0; // 49, 54, or 70

        match cmd {
            7 => { v5 = 2210; path = 49; }
            8 => { v5 = 680; path = 54; }
            9 => {
                let nib = self.read_nibble();
                if nib < 0 { return; }
                v5 = CURVE_TABLE_9[nib as usize & 0x0F];
                path = 49;
            }
            10 => {
                let nib = self.read_nibble();
                if nib < 0 { return; }
                v5 = CURVE_TABLE_10[nib as usize & 0x0F];
                path = 54;
            }
            11 => {
                let b = self.read_byte_aligned() as u32;
                v5 = (b & 3) + 4 * ((b & 0x3C) + 4 * (b & 0xC0));
                path = 49;
            }
            12 => {
                let b = self.read_byte_aligned() as u32;
                v5 = b * 4;
                path = 54;
            }
            13 => {
                let nib = self.read_nibble();
                if nib < 0 { return; }
                v5 = CURVE_TABLE_13[nib as usize & 0x0F];
                path = 70;
            }
            14 => {
                let b = self.read_byte_aligned() as u32;
                v5 = calculate_encoding_14(b);
                path = 70;
            }
            15 => {
                let hi_nibble = self.read_nibble();
                if hi_nibble < 0 { return; }
                let low_byte = self.read_byte_aligned() as u32;
                v5 = low_byte + ((hi_nibble as u32) << 8);
                path = 70;
            }
            _ => { return; }
        }

        let mut v44 = self.cur_x;
        let mut v45 = self.cur_y;
        let v46: i16;
        let v47: i16;
        let v48_0: i16;
        let v48_1: i16;

        let start_x = self.cur_x;
        let start_y = self.cur_y;

        if path == 49 {
            self.read_encoded_coord_pair_into((v5 & 0xF) as i32, &mut v44, &mut v45);

            let mut c2x = self.orus_lookup(0, 0);
            let mut c2y = v45;

            self.read_encoded_coord_pair_into(((v5 >> 4) & 0xF) as i32, &mut c2x, &mut c2y);

            let mut ex = c2x;
            let mut ey = self.orus_lookup(1, 0);

            self.read_encoded_coord_pair_into(((v5 >> 8) & 0xF) as i32, &mut ex, &mut ey);

            v46 = c2x; v47 = c2y;
            v48_0 = ex; v48_1 = ey;
        } else if path == 54 {
            self.read_encoded_coord_pair_into((v5 & 0xF) as i32, &mut v44, &mut v45);

            let mut c2x = v44;
            let mut c2y = self.orus_lookup(1, 0);

            self.read_encoded_coord_pair_into(((v5 >> 4) & 0xF) as i32, &mut c2x, &mut c2y);

            let mut ex = self.orus_lookup(0, 0);
            let mut ey = c2y;

            self.read_encoded_coord_pair_into(((v5 >> 8) & 0xF) as i32, &mut ex, &mut ey);

            v46 = c2x; v47 = c2y;
            v48_0 = ex; v48_1 = ey;
        } else {
            v44 = v44.wrapping_add(self.cur_x.wrapping_sub(self.prev_x));
            v45 = v45.wrapping_add(self.cur_y.wrapping_sub(self.prev_y));

            self.read_encoded_coord_pair_into((v5 & 0xF) as i32, &mut v44, &mut v45);

            let mut c2x = v44;
            let mut c2y = v45;
            self.read_encoded_coord_pair_into(((v5 >> 4) & 0xF) as i32, &mut c2x, &mut c2y);

            let mut ex = c2x;
            let mut ey = c2y;
            self.read_encoded_coord_pair_into(((v5 >> 8) & 0xF) as i32, &mut ex, &mut ey);

            v46 = c2x; v47 = c2y;
            v48_0 = ex; v48_1 = ey;
        }

        // Determine curve type using cross-product
        let cross = (v44 as i32 - start_x as i32) * (v48_1 as i32 - v45 as i32)
                  - (v45 as i32 - start_y as i32) * (v48_0 as i32 - v44 as i32);

        if cross == 0 {
            // Collinear - emit as line
            self.add_curve_points(start_x, start_y, v44, v45, v46, v47, v48_0, v48_1);
        } else {
            // Cubic bezier
            self.add_cubic_bezier(v44, v45, v46, v47, v48_0, v48_1);
        }

        self.cur_x = v48_0;
        self.cur_y = v48_1;
    }

    /// Read encoded coordinate pair
    fn read_encoded_coord_pair_into(&mut self, enc: i32, out_x: &mut i16, out_y: &mut i16) {
        let x_enc = enc & 3;
        let y_enc = (enc >> 2) & 3;

        if x_enc != 0 {
            *out_x = self.read_encoded_coord_value(x_enc, 0, self.cur_x);
        }

        self.prev_x = self.cur_x;
        self.cur_x = *out_x;

        if y_enc != 0 {
            *out_y = self.read_encoded_coord_value(y_enc, 1, self.cur_y);
        }

        self.prev_y = self.cur_y;
        self.cur_y = *out_y;
    }

    /// Read a single encoded coordinate value
    fn read_encoded_coord_value(&mut self, enc: i32, axis: i32, current: i16) -> i16 {
        match enc {
            0 => current,
            1 => {
                // Nibble delta (-8 to +7)
                let n = self.read_nibble();
                if n >= 0 {
                    (n as i16).wrapping_add(current).wrapping_sub(8)
                } else {
                    current
                }
            }
            2 => {
                // Byte value with orus or direct delta
                let b = self.read_byte_aligned();
                let sb = b as i8;
                if sb >= -8 && sb < 8 {
                    let direction: i16 = if (sb & 0x80u8 as i8) == 0 {
                        sb as i16 + 1
                    } else {
                        sb as i16
                    };
                    self.orus_lookup(axis, direction)
                } else {
                    current.wrapping_add(sb as i16)
                }
            }
            3 => {
                // 12/16-bit signed delta
                let hi = self.read_byte_aligned();
                self.nibble_high = !self.nibble_high;

                let lo = if self.pos >= self.data.len() {
                    -1
                } else if self.nibble_high {
                    (self.data[self.pos] >> 4) as i32
                } else {
                    let v = (self.data[self.pos] & 0x0F) as i32;
                    self.pos += 1;
                    v
                };

                if lo >= 0 {
                    let d12 = ((hi as i8 as i32) << 4) | lo;
                    if d12 >= -128 && d12 < 128 {
                        let extra = self.read_inline_extra_byte();
                        let d16 = (d12 << 8) | (extra & 0xFF);
                        current.wrapping_add(d16 as i16)
                    } else {
                        current.wrapping_add(d12 as i16)
                    }
                } else {
                    current
                }
            }
            _ => current,
        }
    }

    /// OrusLookup - control point coordinate lookup
    fn orus_lookup(&self, axis: i32, direction: i16) -> i16 {
        let ctrl = if axis == 1 { &self.ctrl_y } else { &self.ctrl_x };
        let current = if axis == 1 { self.cur_y } else { self.cur_x };
        let main_len = if axis == 1 { self.main_ctrl_y_len } else { self.main_ctrl_x_len };
        let count = ctrl.len().min(main_len);

        if count == 0 {
            // When count==0, return current UNCHANGED
            // Command 0 (small delta via orus lookup) is a NO-OP for orus=0 glyphs
            return current;
        }

        let previous = if axis == 1 { self.prev_y } else { self.prev_x };

        // Direction=0: infer direction from current vs previous (matches C# OrusLookup)
        let mut direction = direction;
        if direction == 0 {
            if current < previous {
                direction = -1;
            } else if current == previous {
                return current;
            } else {
                direction = 1;
            }
        }

        // Forward/backward search based on direction
        if direction > 0 {
            // Forward search
            let mut v9 = 0;
            while v9 < count && ctrl[v9] <= current {
                v9 += 1;
            }
            if v9 >= count {
                return current;
            }
            let v8 = (v9 as i32 + direction as i32 - 1).min(count as i32 - 1).max(0);
            ctrl[v8 as usize]
        } else {
            // Backward search
            let mut search_idx = count as i32;
            while { search_idx -= 1; search_idx >= 0 } {
                if ctrl[search_idx as usize] < current {
                    let v8 = (search_idx + direction as i32 + 1).max(0);
                    return ctrl[v8 as usize];
                }
            }
            current
        }
    }

    /// Transform a coordinate pair through zone tables
    fn transform_point(&mut self, in_x: i16, in_y: i16) -> (i32, i32) {
        let mut coord_x = in_x;
        let mut coord_y = in_y;

        // Apply interpolation if flag620 bits are set (only for orus > 0)
        if self.x_orus_count > 0 && (self.flag_620 & 1) != 0 {
            coord_x = self.interpolate_coord(coord_x, 0);
        }
        if self.y_orus_count > 0 && (self.flag_620 & 2) != 0 {
            coord_y = self.interpolate_coord(coord_y, 1);
        }

        let out_x = self.transform_coordinate(coord_x, 0);
        let out_y = self.transform_coordinate(coord_y, 1);
        (out_x, out_y)
    }

    /// Coordinate interpolation
    fn interpolate_coord(&mut self, coord: i16, axis: i32) -> i16 {
        let (ctrl, scaled) = if axis == 1 {
            (&self.ctrl_y, &self.scaled_y)
        } else {
            (&self.ctrl_x, &self.scaled_x)
        };
        let count = ctrl.len();
        if count == 0 {
            return coord;
        }

        let mut v8 = coord;
        let v9 = ctrl[0];

        if v8 <= v9 {
            v8 = v8.wrapping_add(scaled[0].wrapping_sub(v9));
        } else if v8 >= ctrl[count - 1] {
            v8 = v8.wrapping_add(scaled[count - 1].wrapping_sub(ctrl[count - 1]));
        } else {
            let mut i = 1usize;
            while i < count && v8 > ctrl[i] {
                i += 1;
            }

            if i < count && v8 == ctrl[i] {
                v8 = scaled[i];
            } else if i < count {
                let ctrl_prev = ctrl[i - 1];
                let ctrl_cur = ctrl[i];
                let scaled_prev = scaled[i - 1];
                let scaled_cur = scaled[i];

                let ctrl_delta = ctrl_cur as i32 - ctrl_prev as i32;
                let scaled_delta = scaled_cur as i32 - scaled_prev as i32;
                let input_delta = v8 as i32 - ctrl_prev as i32;

                if ctrl_delta != 0 {
                    let numerator = (ctrl_delta >> 1) + scaled_delta * input_delta;
                    v8 = (scaled_prev as i32 + (numerator / ctrl_delta)) as i16;
                } else {
                    v8 = scaled_prev;
                }
            }
        }

        let mut last_interp = if axis == 1 { self.last_interp_y } else { self.last_interp_x };
        let mut interp_accum = if axis == 1 { self.interp_accum_y } else { self.interp_accum_x };

        if coord != last_interp {
            last_interp = coord;
            let adjustment = self.read_hint_adjustment();
            interp_accum = interp_accum.wrapping_add(adjustment);
        }

        if axis == 1 {
            self.last_interp_y = last_interp;
            self.interp_accum_y = interp_accum;
        } else {
            self.last_interp_x = last_interp;
            self.interp_accum_x = interp_accum;
        }

        v8.wrapping_add(interp_accum)
    }

    /// Read a nibble from the hint stream (backwards)
    fn hint_read_nibble(&mut self) -> i32 {
        if self.hint_pos < 0 || self.hint_pos as usize >= self.data.len() {
            return 0;
        }

        if self.hint_nibble_high == 1 {
            self.hint_nibble_high = 0;
            return (self.data[self.hint_pos as usize] & 0x0F) as i32;
        }

        self.hint_nibble_high = 1;
        let result = (self.data[self.hint_pos as usize] >> 4) as i32;
        self.hint_pos -= 1;
        result
    }

    /// Read hint adjustment value
    fn read_hint_adjustment(&mut self) -> i16 {
        if self.hint_repeat_count > 0 {
            self.hint_repeat_count -= 1;
            return 0;
        }

        if self.hint_pos < 0 {
            return 0;
        }

        let cmd = self.hint_read_nibble();
        match cmd {
            0 => 0,
            1 => {
                self.hint_repeat_count = self.hint_read_nibble() + 2;
                0
            }
            2 | 3 | 4 => (cmd - 5) as i16,
            5 | 6 | 7 => (cmd - 4) as i16,
            8 => (self.hint_read_nibble() - 35) as i16,
            9 => (self.hint_read_nibble() - 19) as i16,
            10 => (self.hint_read_nibble() + 4) as i16,
            11 => (self.hint_read_nibble() + 20) as i16,
            12 => {
                let v6 = self.hint_read_nibble();
                (self.hint_read_nibble() + 16 * v6 - 291) as i16
            }
            13 => {
                let v7 = self.hint_read_nibble();
                (self.hint_read_nibble() + 16 * v7 + 36) as i16
            }
            14 => {
                let v8 = self.hint_read_nibble();
                let v9 = self.hint_read_nibble();
                let v10 = self.hint_read_nibble();
                let mut result = 16 * (16 * (16 * v8 + v9) + v10);
                if result >= 0x8000 {
                    result -= 0x10000;
                }
                (result >> 4) as i16
            }
            _ => {
                let v10 = self.hint_read_nibble();
                let v11 = 16 * v10 + self.hint_read_nibble();
                let v12 = self.hint_read_nibble();
                (16 * (16 * v11 + v12) + self.hint_read_nibble()) as i16
            }
        }
    }

    /// Apply zone transformation to a single coordinate.
    fn transform_coordinate(&mut self, coord: i16, axis: i32) -> i32 {
        let zones = if axis == 1 { &self.zones_y } else { &self.zones_x };
        let scalars = if axis == 1 { &self.scalars_y } else { &self.scalars_x };
        let offsets = if axis == 1 { &self.offsets_y } else { &self.offsets_x };
        let n_zones = if axis == 1 { self.n_zones_y } else { self.n_zones_x };
        let shift = self.shift_difference as i32;

        if n_zones == 0 || zones.is_empty() {
            let scale = if axis == 1 { self.font_scale_y } else { self.font_scale_x };
            let offset = if axis == 1 { self.font_offset_y } else { self.font_offset_x };
            return ((offset as i64 + coord as i64 * scale as i64) >> shift) as i32;
        }

        // Find zone
        let mut i = 0;
        while i < n_zones as usize && coord > zones[i] {
            i += 1;
        }
        if i >= zones.len() { i = zones.len() - 1; }

        ((offsets[i] as i64 + coord as i64 * scalars[i] as i64) >> shift) as i32
    }

    /// Apply transformation flags (includes case 4 matrix path)
    fn apply_transform_flags_with_raw(&self, raw_x: i16, raw_y: i16, x: i32, y: i32) -> (i32, i32) {
        let mut out_x = match self.x_transform_flag {
            0 => x,
            1 => -x,
            2 => y,
            3 => -y,
            _ => {
                let v = self.rounding_bias_2128
                    + self.accumulated_2156
                    + raw_y as i32 * self.scaled_matrix_c as i32
                    + raw_x as i32 * self.scaled_matrix_a as i32;
                v >> self.shift_difference
            }
        };
        let mut out_y = match self.y_transform_flag {
            0 => y,
            1 => -y,
            2 => x,
            3 => -x,
            _ => {
                let v = self.rounding_bias_2128
                    + self.accumulated_2164
                    + raw_y as i32 * self.scaled_matrix_d as i32
                    + raw_x as i32 * self.scaled_matrix_b as i32;
                v >> self.shift_difference
            }
        };

        // Apply secondaryScale to convert to pixel space. The grid-fit path
        // keeps sub-pixel units here; `orus_zero_scale` divides them out in
        // the f32 conversion so snapped fractions survive.
        if !self.grid_fit {
            out_x >>= self.secondary_scale;
            out_y >>= self.secondary_scale;
        } else if self.fit_2192 != 0 {
            // Re-centering correction: shift the fitted glyph
            // along the advance by the average outer-stem snap displacement.
            match self.x_transform_flag {
                0 => out_x += self.fit_2192,
                1 => out_x -= self.fit_2192,
                _ => {}
            }
        }

        (out_x, out_y)
    }

    // ===================== Grid-fit (hinted) path =====================
    // The scaled control tables are not a
    // plain linear scale of the ctrl orus values: control values come in PAIRS
    // (stem edges), and each pair's WIDTH is snapped — to the font's stem-snap
    // table, to a minimum of one pixel, and to the pixel grid — while its
    // POSITION is grid-rounded (X: midpoint; Y: blue-zone capture of either
    // edge). Everything runs in "scaled sub-pixel units": one pixel is
    // `scaled_pow2` units, `shift_difference` converts orus·scale products
    // down into them.

    /// Round a scaled value to the pixel grid: `mask & (half + v)`.
    #[inline]
    fn fit_round(&self, v: i32) -> i16 {
        ((v.wrapping_add(self.scaled_pow2_half as i32)) as i16) & self.scaled_pow2_neg
    }

    /// Build the per-size blue-zone and stem-snap tables (once per size).
    fn fit_build_tables(&mut self) {
        if self.fit_tables_ready {
            return;
        }
        self.fit_tables_ready = true;

        let shift = self.shift_difference as i32;
        let quarter = (self.scaled_pow2_half >> 1) as i32;

        // ---- Blue zones (Y axis); mode bit 8 disables them ----
        let n_pairs = if (self.fit_mode_y & 8) != 0 {
            0
        } else {
            self.fit_blue_values.len() / 2
        };
        let scale_y = self.font_scale_y as i32;
        let offset_y = self.font_offset_y;
        let mut zones: Vec<[i16; 3]> = Vec::with_capacity(n_pairs);
        for i in 0..n_pairs {
            let lo = self.fit_blue_values[2 * i] as i32;
            let hi = self.fit_blue_values[2 * i + 1] as i32;
            // The FIRST zone's snap target is the scaled position of orus 0
            // (the baseline), not of its own lower blue value.
            let value_src = if i == 0 { 0 } else { lo };
            let value = ((offset_y + scale_y * value_src) >> shift) as i16;
            let min = ((offset_y + scale_y * (lo - self.fit_blue_fuzz as i32)) >> shift) as i16;
            let max = ((offset_y + scale_y * (hi + self.fit_blue_fuzz as i32)) >> shift) as i16;
            zones.push([min, max, value]);
        }
        // Overlap fix-ups: each zone reaches at most a quarter pixel past its
        // snap value, and adjacent zones split the gap at the midpoint.
        if !zones.is_empty() {
            let v = zones[0][2] as i32 - quarter;
            if (zones[0][0] as i32) > v {
                zones[0][0] = v as i16;
            }
            for i in 1..zones.len() {
                let mid = ((zones[i][0] as i32 + zones[i - 1][1] as i32) >> 1) as i16;
                let mut up = (zones[i - 1][2] as i32 + quarter) as i16;
                if up > mid {
                    up = mid;
                }
                if zones[i - 1][1] < up {
                    zones[i - 1][1] = up;
                }
                let mut dn = (zones[i][2] as i32 - quarter) as i16;
                if dn < mid {
                    dn = mid;
                }
                if zones[i][0] > dn {
                    zones[i][0] = dn;
                }
            }
            let last = zones.len() - 1;
            let up = (zones[last][2] as i32 + quarter) as i16;
            if zones[last][1] < up {
                zones[last][1] = up;
            }
        }
        self.fit_blue_zones = zones;

        // Overshoot suppression below the blue ppem threshold (field 2190):
        // scale_y * em < blue_ppem << coord_shift.
        let em = self.outline_resolution as i32;
        self.fit_blue_suppress =
            scale_y * em < (self.fit_blue_ppem as i32) << self.coord_shift;
        // word 230 = 2731 >> (12 - secondary_scale): ~0.667px asymmetric bias
        // used when re-rounding a top edge relative to its zone value.
        let sec = self.secondary_scale as i32;
        self.fit_word230 = (2731i32 >> (12 - sec).clamp(0, 15)) as i16;
        // word 228 = scale_y * word126 >> shift, word126 = 0.007 * em orus.
        let word126 = ((459i64 * em as i64 + 0x8000) >> 16) as i32;
        self.fit_word228 = ((scale_y * word126) >> shift) as i16;

        // ---- Stem-snap tables, X then Y ----
        let snap_v = std::mem::take(&mut self.fit_stem_snap_v);
        let snap_h = std::mem::take(&mut self.fit_stem_snap_h);
        self.fit_snap_x = self.fit_build_snap_table(self.font_scale_x, &snap_v, self.std_vw as i32);
        self.fit_snap_y = self.fit_build_snap_table(self.font_scale_y, &snap_h, self.std_hw as i32);
        self.fit_stem_snap_v = snap_v;
        self.fit_stem_snap_h = snap_h;
    }

    /// One axis of the stem-snap table: snap records `[min, max, value]` in
    /// scaled units, from the font's stem-snap widths plus a wider band around
    /// the standard stem width.
    fn fit_build_snap_table(&self, scale: i16, snaps: &[i16], std_w: i32) -> Vec<[i16; 3]> {
        let shift = self.shift_difference as i32;
        let quarter = (self.scaled_pow2_half >> 1) as i32;
        let mut recs: Vec<[i16; 3]> = Vec::with_capacity(snaps.len() + 1);

        for &snap in snaps {
            let v11 = ((self.rounding_bias_2128 + scale as i32 * snap as i32) >> shift) as i16;
            let mut lo = (v11 as i32 - quarter) as i16;
            if let Some(prev) = recs.last_mut() {
                if prev[1] > lo {
                    if prev[1] > v11 {
                        prev[1] = v11;
                    }
                    if lo < prev[2] {
                        lo = prev[2];
                    }
                    lo = ((lo as i32 + prev[1] as i32) >> 1) as i16;
                    prev[1] = lo;
                }
            }
            recs.push([lo, (v11 as i32 + quarter) as i16, v11]);
        }

        if std_w > 0 {
            let sh = (16 - self.secondary_scale as i32).clamp(0, 15);
            let v15 = ((self.rounding_bias_2128 + std_w * scale as i32) >> shift) as i16;
            let rounded = if v15 >= self.scaled_pow2 {
                self.fit_round(v15 as i32)
            } else {
                self.scaled_pow2
            };
            // Band ~[-0.36, +0.60]px around the linear StdW, clamped against
            // the grid-rounded StdW by [-0.725, +0.875]px.
            let mut lo = v15 as i32 - (23593 >> sh);
            lo = lo.max(rounded as i32 - (47513 >> sh));
            lo = lo.min(v15 as i32);
            let mut hi = v15 as i32 + (39322 >> sh);
            hi = hi.min(rounded as i32 + (57344 >> sh));
            hi = hi.max(v15 as i32);
            let mut lo = lo as i16;
            let mut hi = hi as i16;

            // Records whose value falls inside the StdW band merge into it.
            let mut kept: Vec<[i16; 3]> = Vec::with_capacity(recs.len() + 1);
            for rec in recs {
                if rec[2] < lo || rec[2] > hi {
                    kept.push(rec);
                } else {
                    if rec[0] < lo {
                        lo = rec[0];
                    }
                    if rec[1] > hi {
                        hi = rec[1];
                    }
                }
            }
            // Insert the StdW record sorted by value; clamp against neighbors.
            let ins = kept.iter().position(|r| r[2] > v15).unwrap_or(kept.len());
            if ins > 0 && lo < kept[ins - 1][1] {
                lo = kept[ins - 1][1];
            }
            if ins < kept.len() && hi > kept[ins][0] {
                hi = kept[ins][1];
            }
            kept.insert(ins, [lo, hi, v15]);
            kept
        } else {
            recs
        }
    }

    /// Snap one scaled stem width: stem-snap table, then a 1px minimum, then
    /// the pixel grid (shared tail of both axes' pair fitting).
    #[inline]
    fn fit_snap_width(&self, mut w: i16, snap_table: &[[i16; 3]]) -> i16 {
        for rec in snap_table {
            if w < rec[0] {
                break;
            }
            if w <= rec[1] {
                w = rec[2];
                break;
            }
        }
        if w >= self.scaled_pow2 {
            self.fit_round(w as i32)
        } else {
            self.scaled_pow2
        }
    }

    /// Snap X control pairs (stems) into `scaled_x`.
    /// `adjacent` < 0 places each stem at its grid-rounded scaled midpoint;
    /// otherwise the stem is placed relative to the already-fitted pair at
    /// that index.
    fn fit_snap_pairs_x(&mut self, start: usize, end: usize, adjacent: i32) {
        let shift = self.shift_difference as i32;
        let mut i = start;
        while i + 1 < end.min(self.ctrl_x.len()) {
            let a = self.ctrl_x[i] as i32;
            let b = self.ctrl_x[i + 1] as i32;
            let w = ((self.rounding_bias_2128 + self.font_scale_x as i32 * (b - a)) >> shift) as i16;
            let snapped = self.fit_snap_width(w, &self.fit_snap_x);
            let (mid, base) = if adjacent < 0 || (adjacent as usize) + 1 >= self.scaled_x.len() {
                ((b + a) >> 1,
                 self.font_offset_x - (((snapped as i32) << shift) >> 1))
            } else {
                let j = adjacent as usize;
                ((b + a - self.ctrl_x[j] as i32 - self.ctrl_x[j + 1] as i32) >> 1,
                 ((self.scaled_x[j + 1] as i32 + self.scaled_x[j] as i32 - snapped as i32) << shift) >> 1)
            };
            let pos = self.fit_round((base + self.font_scale_x as i32 * mid) >> shift);
            self.scaled_x[i] = pos;
            self.scaled_x[i + 1] = pos.wrapping_add(snapped);
            i += 2;
        }
        // Odd trailing entry: plain linear scale.
        if i < end.min(self.ctrl_x.len()) {
            self.scaled_x[i] =
                ((self.font_offset_x + self.font_scale_x as i32 * self.ctrl_x[i] as i32) >> shift) as i16;
        }
    }

    /// Snap Y control pairs into `scaled_y`, capturing edges that land in a
    /// blue zone.
    fn fit_snap_pairs_y(&mut self, start: usize, end: usize, adjacent: i32) {
        let shift = self.shift_difference as i32;
        let n_zones = self.fit_blue_zones.len();
        let mut bot_cur = 0usize;
        let mut top_cur = 0usize;
        let mut i = start;
        while i + 1 < end.min(self.ctrl_y.len()) {
            let a = self.ctrl_y[i] as i32;
            let b = self.ctrl_y[i + 1] as i32;
            let mut cases = 0u8;
            let mut bot_snap: i16 = 0;
            let mut top_snap: i16 = 0;

            // Bottom edge against the blue zones (cursor only advances —
            // pairs arrive sorted by position).
            let bpos = ((self.font_offset_y + a * self.font_scale_y as i32) >> shift) as i16;
            while bot_cur < n_zones && bpos > self.fit_blue_zones[bot_cur][1] {
                bot_cur += 1;
            }
            if bot_cur < n_zones && bpos >= self.fit_blue_zones[bot_cur][0] {
                cases |= 1;
                let z = self.fit_blue_zones[bot_cur];
                let v12 = if self.fit_blue_suppress {
                    0
                } else {
                    self.fit_round(z[2] as i32 - bpos as i32)
                };
                bot_snap = self.fit_round(z[2] as i32).wrapping_sub(v12);
            }

            // Top edge.
            let tpos = ((self.font_offset_y + b * self.font_scale_y as i32) >> shift) as i16;
            while top_cur < n_zones && tpos > self.fit_blue_zones[top_cur][1] {
                top_cur += 1;
            }
            if top_cur < n_zones && tpos >= self.fit_blue_zones[top_cur][0] {
                cases |= 2;
                let z = self.fit_blue_zones[top_cur];
                let v18 = if self.fit_blue_suppress {
                    0
                } else {
                    let d = tpos.wrapping_sub(z[2]);
                    if d >= self.scaled_pow2_half || d < self.fit_word228 {
                        self.fit_round(d as i32)
                    } else {
                        self.scaled_pow2
                    }
                };
                let anchored =
                    ((self.fit_word230 as i32 + z[2] as i32) as i16) & self.scaled_pow2_neg;
                top_snap = v18.wrapping_add(anchored);
            }

            // Snapped height.
            let hsnap: i16 = if a == b {
                0
            } else {
                let h = ((self.rounding_bias_2128 + self.font_scale_y as i32 * (b - a)) >> shift) as i16;
                self.fit_snap_width(h, &self.fit_snap_y)
            };

            match cases {
                0 => {
                    let (mid, base) = if adjacent < 0 || (adjacent as usize) + 1 >= self.scaled_y.len() {
                        ((a + b) >> 1,
                         self.font_offset_y - (((hsnap as i32) << shift) >> 1))
                    } else {
                        let j = adjacent as usize;
                        ((a + b - self.ctrl_y[j] as i32 - self.ctrl_y[j + 1] as i32) >> 1,
                         ((self.scaled_y[j + 1] as i32 + self.scaled_y[j] as i32 - hsnap as i32) << shift) >> 1)
                    };
                    let pos = self.fit_round((base + self.font_scale_y as i32 * mid) >> shift);
                    self.scaled_y[i] = pos;
                    self.scaled_y[i + 1] = pos.wrapping_add(hsnap);
                }
                1 => {
                    self.scaled_y[i] = bot_snap;
                    self.scaled_y[i + 1] = bot_snap.wrapping_add(hsnap);
                }
                2 => {
                    self.scaled_y[i] = top_snap.wrapping_sub(hsnap);
                    self.scaled_y[i + 1] = top_snap;
                }
                _ => {
                    self.scaled_y[i] = bot_snap;
                    self.scaled_y[i + 1] = top_snap;
                }
            }
            i += 2;
        }
        if i < end.min(self.ctrl_y.len()) {
            self.scaled_y[i] =
                ((self.font_offset_y + self.font_scale_y as i32 * self.ctrl_y[i] as i32) >> shift) as i16;
        }
    }

    /// Fill the scaled tables from the ctrl pairs, append any extra
    /// controlled edges from the glyph's extra-item block (types 1 and 2),
    /// then build the zone tables from the fitted result.
    fn fit_scaled_and_zones(&mut self, extra_items: Option<(usize, usize)>) {
        self.fit_build_tables();
        let shift = self.shift_difference as i32;

        let n_main_x = self.ctrl_x.len();
        let n_main_y = self.ctrl_y.len();
        self.scaled_x = vec![0; n_main_x];
        self.scaled_y = vec![0; n_main_y];

        // Axis fitting is skipped (plain linear scale) when the mode byte has
        // bit 4 set, or when the axis has no usable scale (full-matrix case).
        let fit_x = (self.fit_mode_x & 4) == 0 && self.font_scale_x != 0 && self.x_transform_flag <= 3;
        let fit_y = (self.fit_mode_y & 4) == 0 && self.font_scale_y != 0 && self.y_transform_flag <= 3;

        if fit_x {
            self.fit_snap_pairs_x(0, n_main_x, -1);
        } else {
            for i in 0..n_main_x {
                self.scaled_x[i] =
                    ((self.font_offset_x + self.font_scale_x as i32 * self.ctrl_x[i] as i32) >> shift) as i16;
            }
        }
        if fit_y {
            self.fit_snap_pairs_y(0, n_main_y, -1);
        } else {
            for i in 0..n_main_y {
                self.scaled_y[i] =
                    ((self.font_offset_y + self.font_scale_y as i32 * self.ctrl_y[i] as i32) >> shift) as i16;
            }
        }

        // ---- Extra items: additional controlled edges ----
        let mut extras_x = false;
        let mut extras_y = false;
        if let Some((start, end)) = extra_items {
            let data: Vec<u8> = self.data[start..end.min(self.data.len())].to_vec();
            let mut p = 0usize;
            if p < data.len() {
                let n_items = data[p] as usize;
                p += 1;
                for _ in 0..n_items {
                    if p + 2 > data.len() {
                        break;
                    }
                    let item_len = data[p] as usize;
                    let item_type = data[p + 1];
                    let item = &data[p + 2..(p + 2 + item_len).min(data.len())];
                    p += item_len + 2;
                    match item_type {
                        1 => {
                            let (ex, ey) = self.fit_extra_pairs(item, fit_x, fit_y);
                            extras_x |= ex;
                            extras_y |= ey;
                        }
                        2 => {
                            let (ex, ey) = self.fit_extra_edges(item, fit_x, fit_y);
                            extras_x |= ex;
                            extras_y |= ey;
                        }
                        _ => {}
                    }
                }
            }
        }

        // ---- Zone tables from the fitted ctrl/scaled tables ----
        self.initialize_zone_tables_fitted(0, extras_x);
        self.initialize_zone_tables_fitted(1, extras_y);

        // ---- Re-centering correction ----
        // Average (linear - snapped) displacement of the outermost MAIN X stem
        // edges that fall inside the advance band [0, setWidth]: the first
        // pair START in band and the last pair END in band.
        self.fit_2192 = 0;
        if fit_x && self.fit_set_width_orus != 0 {
            let (band_lo, band_hi) = if self.fit_set_width_orus < 0 {
                (self.fit_set_width_orus, 0)
            } else {
                (0, self.fit_set_width_orus)
            };
            let mut lo_idx: i32 = -1;
            let mut hi_idx: i32 = -1;
            let mut k = 0usize;
            while k + 1 < n_main_x {
                let a = self.ctrl_x[k] as i32;
                let b = self.ctrl_x[k + 1] as i32;
                if lo_idx < 0 && a >= band_lo && a <= band_hi {
                    lo_idx = k as i32;
                }
                if b >= band_lo && b <= band_hi {
                    hi_idx = (k + 1) as i32;
                }
                k += 2;
            }
            let mut sum: i32 = 0;
            let mut n: i32 = 0;
            for idx in [lo_idx, hi_idx] {
                if idx >= 0 {
                    let i = idx as usize;
                    let linear = (self.rounding_bias_2128
                        + self.font_scale_x as i32 * self.ctrl_x[i] as i32)
                        >> shift;
                    sum += linear - self.scaled_x[i] as i32;
                    n += 1;
                }
            }
            if n > 0 {
                // Director applies this at BLIT time to the already-rasterized
                // glyph's origin (its 16.16 bbox position), so the
                // ink only ever moves by whole pixels. We bake the shift into
                // the contours instead, so quantize it to the pixel grid —
                // a fractional shift would re-sample the fitted outline.
                self.fit_2192 = self.fit_round((sum + (n >> 1)) / n) as i32;
            }
        }
    }

    /// Extra-item type 1 — additional (start, end) orus stem pairs,
    /// big-endian, appended to the ctrl tables and fitted against the nearest
    /// overlapping existing pair. Returns which axes grew.
    fn fit_extra_pairs(&mut self, item: &[u8], fit_x: bool, fit_y: bool) -> (bool, bool) {
        let mut p = 0usize;
        let mut grew = (false, false);
        for axis in 0..2 {
            if p >= item.len() {
                break;
            }
            let count = item[p] as usize;
            p += 1;
            let enabled = if axis == 0 { fit_x } else { fit_y };
            if !enabled {
                p += 4 * count;
                continue;
            }
            for _ in 0..count {
                if p + 4 > item.len() {
                    break;
                }
                let a = i16::from_be_bytes([item[p], item[p + 1]]);
                let b = i16::from_be_bytes([item[p + 2], item[p + 3]]);
                p += 4;
                let (ctrl, scaled) = if axis == 0 {
                    (&mut self.ctrl_x, &mut self.scaled_x)
                } else {
                    (&mut self.ctrl_y, &mut self.scaled_y)
                };
                let base = ctrl.len();
                // Skip a truncated/odd table — pairs must start on an even slot.
                if base % 2 != 0 {
                    break;
                }
                ctrl.push(a);
                ctrl.push(b);
                scaled.push(0);
                scaled.push(0);
                // Find an existing pair overlapping the new one: first pair
                // whose end >= new start, if the new end also exceeds its
                // start; otherwise anchor to pair 0.
                let mut adj = 0usize;
                let (ctrl, n) = if axis == 0 {
                    (&self.ctrl_x, base)
                } else {
                    (&self.ctrl_y, base)
                };
                let mut k = 0usize;
                while k + 1 < n {
                    if a <= ctrl[k + 1] {
                        break;
                    }
                    k += 2;
                }
                if k + 1 < n && b > ctrl[k] {
                    adj = k;
                }
                if axis == 0 {
                    self.fit_snap_pairs_x(base, base + 2, adj as i32);
                    grew.0 = true;
                } else {
                    self.fit_snap_pairs_y(base, base + 2, adj as i32);
                    grew.1 = true;
                }
            }
        }
        grew
    }

    /// Extra-item type 2 — controlled edges defined as deltas
    /// from an existing ctrl entry. The scaled delta collapses to zero when
    /// smaller than the record's threshold (in 16ths of a pixel).
    fn fit_extra_edges(&mut self, item: &[u8], fit_x: bool, fit_y: bool) -> (bool, bool) {
        let shift = self.shift_difference as i32;
        let mut p = 0usize;
        let mut grew = (false, false);
        for axis in 0..2 {
            if p >= item.len() {
                break;
            }
            let count = item[p] as usize;
            p += 1;
            let enabled = if axis == 0 { fit_x } else { fit_y };
            let scale = if axis == 0 { self.font_scale_x } else { self.font_scale_y } as i32;
            let mut ref_idx: i32 = 0;
            for _ in 0..count {
                if p >= item.len() {
                    break;
                }
                let b = item[p];
                p += 1;
                let threshold: i32;
                let delta: i32;
                if b & 0x80 != 0 {
                    ref_idx = (b & 0x3F) as i32 - 1;
                    if ref_idx < 0 {
                        if p >= item.len() {
                            break;
                        }
                        ref_idx = item[p] as i32;
                        p += 1;
                    }
                    if b & 0x40 != 0 {
                        threshold = 16;
                    } else {
                        if p >= item.len() {
                            break;
                        }
                        threshold = item[p] as i32;
                        p += 1;
                    }
                    if p >= item.len() {
                        break;
                    }
                    let d8 = item[p] as i8;
                    p += 1;
                    if d8 == 0 {
                        if p + 2 > item.len() {
                            break;
                        }
                        delta = i16::from_be_bytes([item[p], item[p + 1]]) as i32;
                        p += 2;
                    } else {
                        delta = d8 as i32;
                    }
                } else {
                    ref_idx += (b >> 4) as i32;
                    threshold = 16;
                    // Sign-extend the low nibble.
                    delta = (((b << 4) as i8) >> 4) as i32;
                }
                if !enabled {
                    continue;
                }
                let scaled_delta = ((self.rounding_bias_2128 + delta * scale) >> shift) as i16;
                let thr = ((threshold << self.secondary_scale) >> 4) as i16;
                let snap = if scaled_delta >= thr || scaled_delta <= thr.wrapping_neg() {
                    self.fit_round(scaled_delta as i32)
                } else {
                    0
                };
                let (ctrl, scaled) = if axis == 0 {
                    (&mut self.ctrl_x, &mut self.scaled_x)
                } else {
                    (&mut self.ctrl_y, &mut self.scaled_y)
                };
                let r = ref_idx as usize;
                if r >= ctrl.len() {
                    continue;
                }
                let new_ctrl = ctrl[r].wrapping_add(delta as i16);
                let new_scaled = scaled[r].wrapping_add(snap);
                ctrl.push(new_ctrl);
                scaled.push(new_scaled);
                if axis == 0 {
                    grew.0 = true;
                } else {
                    grew.1 = true;
                }
            }
        }
        grew
    }

    /// Build the piecewise-linear zone tables from the FITTED
    /// ctrl/scaled tables. Offsets reconstruct the snapped positions exactly
    /// (`scaled << shift`), and the scalar division truncates like the
    /// original — no rounding bias.
    fn initialize_zone_tables_fitted(&mut self, axis: i32, had_extras: bool) {
        let (ctrl, scaled, font_scale, font_offset) = if axis == 1 {
            (self.ctrl_y.clone(), self.scaled_y.clone(), self.font_scale_y, self.font_offset_y)
        } else {
            (self.ctrl_x.clone(), self.scaled_x.clone(), self.font_scale_x, self.font_offset_x)
        };
        let shift = self.shift_difference as i32;
        let bias = self.rounding_bias_2128;

        let mut zones: Vec<i16> = Vec::new();
        let mut scalars: Vec<i16> = Vec::new();
        let mut offsets: Vec<i32> = Vec::new();
        let n_ctrl = ctrl.len();

        if n_ctrl == 0 {
            scalars.push(font_scale);
            offsets.push(font_offset + bias);
            zones.push(i16::MAX);
        } else {
            let mut order: Vec<usize> = (0..n_ctrl).collect();
            if had_extras {
                order.sort_by_key(|&i| ctrl[i]);
            }
            let first = order[0];
            let mut last_ctrl = ctrl[first];
            let mut last_scaled = scaled[first];
            scalars.push(font_scale);
            offsets.push(bias + ((last_scaled as i32) << shift) - font_scale as i32 * last_ctrl as i32);
            zones.push(last_ctrl);

            for &idx in order.iter().skip(1) {
                let d = ctrl[idx] as i32 - last_ctrl as i32;
                if d <= 0 {
                    continue;
                }
                let scalar = (((scaled[idx] as i32 - last_scaled as i32) << shift) / d) as i16;
                let offset =
                    bias + ((scaled[idx] as i32) << shift) - ctrl[idx] as i32 * scalar as i32;
                let k = zones.len();
                if scalar == scalars[k - 1] && offset == offsets[k - 1] {
                    zones[k - 1] = ctrl[idx];
                } else {
                    zones.push(ctrl[idx]);
                    scalars.push(scalar);
                    offsets.push(offset);
                }
                last_ctrl = ctrl[idx];
                last_scaled = scaled[idx];
            }

            let final_offset =
                bias + ((last_scaled as i32) << shift) - font_scale as i32 * last_ctrl as i32;
            let k = zones.len();
            if font_scale == scalars[k - 1] && final_offset == offsets[k - 1] {
                zones[k - 1] = i16::MAX;
            } else {
                zones.push(i16::MAX);
                scalars.push(font_scale);
                offsets.push(final_offset);
            }
        }

        let n_zones = zones.len() as i16;
        if axis == 1 {
            self.zones_y = zones;
            self.scalars_y = scalars;
            self.offsets_y = offsets;
            self.n_zones_y = n_zones;
        } else {
            self.zones_x = zones;
            self.scalars_x = scalars;
            self.offsets_x = offsets;
            self.n_zones_x = n_zones;
        }
    }

    /// Compute scaled coordinate arrays
    fn compute_scaled_coordinates(&mut self) {
        self.scaled_x.clear();
        self.scaled_y.clear();

        for &ctrl in &self.ctrl_x {
            let scaled = (self.font_offset_x as i64 + self.font_scale_x as i64 * ctrl as i64) >> self.shift_difference;
            self.scaled_x.push(scaled as i16);
        }

        for &ctrl in &self.ctrl_y {
            let scaled = (self.font_offset_y as i64 + self.font_scale_y as i64 * ctrl as i64) >> self.shift_difference;
            self.scaled_y.push(scaled as i16);
        }

        if self.target_em_px > 0 {
            log(&format!(
                "    ComputeScaledCoordinates: fontScaleX={}, fontScaleY={}, coordShift={}",
                self.font_scale_x, self.font_scale_y, self.coord_shift
            ));
            log(&format!(
                "      fontOffsetX={}, fontOffsetY={}, baseOffset={}",
                self.font_offset_x, self.font_offset_y, self.base_offset
            ));
            if !self.scaled_x.is_empty() {
                log(&format!(
                    "      scaledX={:?}, scaledY={:?}",
                    self.scaled_x, self.scaled_y
                ));
            }
        }
    }

    /// Initialize zone transformation tables
    fn initialize_zone_tables(&mut self) {
        self.initialize_zone_tables_for_axis(0);
        self.initialize_zone_tables_for_axis(1);

        if self.target_em_px > 0 {
            let dc = self.diag_char_code;
            let dch = if dc >= 32 && dc < 127 { dc as u8 as char } else { '?' };
            log(&format!(
                "    ZoneTables[{}'{}']: nZonesX={} nZonesY={} ctrlX={:?} ctrlY={:?}",
                dc, dch, self.n_zones_x, self.n_zones_y, self.ctrl_x, self.ctrl_y
            ));
            if self.n_zones_y > 0 {
                log(&format!(
                    "      zonesY={:?} scalarsY={:?} offsetsY={:?}",
                    self.zones_y, self.scalars_y, self.offsets_y
                ));
            }
        }
    }

    fn initialize_zone_tables_for_axis(&mut self, axis: i32) {
        let (ctrl, scaled, font_scale, font_offset) = if axis == 1 {
            (self.ctrl_y.clone(), self.scaled_y.clone(), self.font_scale_y, self.font_offset_y)
        } else {
            (self.ctrl_x.clone(), self.scaled_x.clone(), self.font_scale_x, self.font_offset_x)
        };

        let mut zones = Vec::new();
        let mut scalars = Vec::new();
        let mut offsets = Vec::new();
        let n_ctrl = ctrl.len();

        if n_ctrl == 0 {
            scalars.push(font_scale);
            offsets.push(font_offset);
            zones.push(i16::MAX);
            let n_zones: i16 = 1;
            if axis == 1 {
                self.zones_y = zones;
                self.scalars_y = scalars;
                self.offsets_y = offsets;
                self.n_zones_y = n_zones;
            } else {
                self.zones_x = zones;
                self.scalars_x = scalars;
                self.offsets_x = offsets;
                self.n_zones_x = n_zones;
            }
            return;
        }

        // Create sorted indices
        let mut sorted_indices: Vec<usize> = (0..n_ctrl).collect();
        sorted_indices.sort_by_key(|&i| ctrl[i]);

        let first_idx = sorted_indices[0];
        let mut last_ctrl = ctrl[first_idx];
        let last_scaled = scaled[first_idx];

        // First zone: use untruncated (font_offset + font_scale * ctrl) instead of
        // truncated (scaled << coord_shift) to preserve rounding bias.
        // The truncation at scaled >> coord_shift << coord_shift loses fractional bits,
        // which shifts the offset and causes coordinates far from ctrl points to be
        // mapped ~1px too low (e.g. x-height of 'u' rendered 1px shorter than 'a').
        let first_zone_untrunc = font_offset as i32 + font_scale as i32 * last_ctrl as i32;
        scalars.push(font_scale);
        offsets.push(self.base_offset + first_zone_untrunc - font_scale as i32 * last_ctrl as i32);
        zones.push(last_ctrl);

        let mut zone_count: usize = 1;
        let mut prev_last_scaled = last_scaled;

        for i in 1..n_ctrl {
            let idx = sorted_indices[i];
            let cur_ctrl = ctrl[idx];
            let cur_scaled = scaled[idx];
            let ctrl_delta = cur_ctrl as i32 - last_ctrl as i32;

            if ctrl_delta > 0 {
                let scaled_delta = cur_scaled as i32 - prev_last_scaled as i32;
                let numerator = scaled_delta << self.shift_difference;
                let round_bias = ctrl_delta >> 1;
                let scalar = if numerator >= 0 {
                    ((numerator + round_bias) / ctrl_delta) as i16
                } else {
                    ((numerator - round_bias) / ctrl_delta) as i16
                };

                // FIX: Use UNTRUNCATED (font_offset + font_scale * ctrl) instead of (scaled << coord_shift).
                // The truncation loses fractional bits (including rounding bias from font_offset),
                // which shifts intermediate coordinates ~1px wrong for pixel fonts.
                let cur_scaled_untruncated = font_offset as i32 + font_scale as i32 * cur_ctrl as i32;
                let offset = self.base_offset + cur_scaled_untruncated - cur_ctrl as i32 * scalar as i32;

                if zone_count > 0 && scalar == scalars[zone_count - 1] && offset == offsets[zone_count - 1] {
                    zones[zone_count - 1] = cur_ctrl;
                } else {
                    scalars.push(scalar);
                    offsets.push(offset);
                    zones.push(cur_ctrl);
                    zone_count += 1;
                }

                last_ctrl = cur_ctrl;
                prev_last_scaled = cur_scaled;
            }
        }

        // Final zone: same untruncated offset fix as first zone
        scalars.push(font_scale);
        let last_zone_untrunc = font_offset as i32 + font_scale as i32 * last_ctrl as i32;
        let final_offset = self.base_offset + last_zone_untrunc - font_scale as i32 * last_ctrl as i32;
        if zone_count > 0 && scalars[zone_count] == scalars[zone_count - 1] && final_offset == offsets[zone_count - 1] {
            zones[zone_count - 1] = i16::MAX;
        } else {
            offsets.push(final_offset);
            zones.push(i16::MAX);
            zone_count += 1;
        }

        let n_zones = zone_count as i16;

        if axis == 1 {
            self.zones_y = zones;
            self.scalars_y = scalars;
            self.offsets_y = offsets;
            self.n_zones_y = n_zones;
        } else {
            self.zones_x = zones;
            self.scalars_x = scalars;
            self.offsets_x = offsets;
            self.n_zones_x = n_zones;
        }
    }

    /// Update prev/cur coordinates and add point
    fn update_coordinates_and_add_point(&mut self, new_x: i16, new_y: i16) {
        self.prev_x = self.cur_x;
        self.prev_y = self.cur_y;

        let (out_x, out_y) = self.transform_point(new_x, new_y);
        let (final_x, final_y) = self.apply_transform_flags_with_raw(new_x, new_y, out_x, out_y);
        let scaled_x = final_x as f32 * self.orus_zero_scale;
        let scaled_y = final_y as f32 * self.orus_zero_scale;

        // Log first point of each contour (move_to) to show orus→pixel mapping
        if self.target_em_px > 0 && self.current_contour.commands.is_empty() && !self.record_strokes {
            log(&format!(
                "    [orus] raw=({},{}) zone=({},{}) final=({},{}) scaled=({:.1},{:.1})",
                new_x, new_y, out_x, out_y, final_x, final_y, scaled_x, scaled_y
            ));
        }

        if self.record_strokes {
            if self.stroke_has_pos {
                self.strokes.push(PfrStroke::line(
                    self.last_stroke_x,
                    self.last_stroke_y,
                    scaled_x,
                    scaled_y,
                    self.current_stroke_width,
                ));
            } else {
                self.stroke_has_pos = true;
            }
            self.last_stroke_x = scaled_x;
            self.last_stroke_y = scaled_y;
            self.cur_x = new_x;
            self.cur_y = new_y;
            return;
        }

        if self.current_contour.commands.is_empty() {
            // Save orus coordinates of the very first point (for compound merge correction)
            if self.first_point_orus_y.is_none() && self.contours.is_empty() {
                self.first_point_orus_x = Some(new_x);
                self.first_point_orus_y = Some(new_y);
            }
            self.current_contour.commands.push(PfrCmd::move_to(scaled_x, scaled_y));
        } else {
            self.current_contour.commands.push(PfrCmd::line_to(scaled_x, scaled_y));
        }

        self.cur_x = new_x;
        self.cur_y = new_y;
    }

    /// Add point at current coordinates (for commands 5-6)
    fn add_point_only(&mut self) {
        let (out_x, out_y) = self.transform_point(self.cur_x, self.cur_y);
        let (final_x, final_y) = self.apply_transform_flags_with_raw(self.cur_x, self.cur_y, out_x, out_y);
        let scaled_x = final_x as f32 * self.orus_zero_scale;
        let scaled_y = final_y as f32 * self.orus_zero_scale;

        if self.record_strokes {
            if self.stroke_has_pos {
                self.strokes.push(PfrStroke::line(
                    self.last_stroke_x,
                    self.last_stroke_y,
                    scaled_x,
                    scaled_y,
                    self.current_stroke_width,
                ));
            } else {
                self.stroke_has_pos = true;
            }
            self.last_stroke_x = scaled_x;
            self.last_stroke_y = scaled_y;
            return;
        }

        if self.current_contour.commands.is_empty() {
            self.current_contour.commands.push(PfrCmd::move_to(scaled_x, scaled_y));
        } else {
            self.current_contour.commands.push(PfrCmd::line_to(scaled_x, scaled_y));
        }
    }

    /// Add cubic bezier curve
    fn add_cubic_bezier(&mut self, ctrl1_x: i16, ctrl1_y: i16, ctrl2_x: i16, ctrl2_y: i16, end_x: i16, end_y: i16) {
        let (tc1x, tc1y) = self.transform_point(ctrl1_x, ctrl1_y);
        let (tc2x, tc2y) = self.transform_point(ctrl2_x, ctrl2_y);
        let (tex, tey) = self.transform_point(end_x, end_y);

        let (fc1x, fc1y) = self.apply_transform_flags_with_raw(ctrl1_x, ctrl1_y, tc1x, tc1y);
        let (fc2x, fc2y) = self.apply_transform_flags_with_raw(ctrl2_x, ctrl2_y, tc2x, tc2y);
        let (fex, fey) = self.apply_transform_flags_with_raw(end_x, end_y, tex, tey);
        let sc1x = fc1x as f32 * self.orus_zero_scale;
        let sc1y = fc1y as f32 * self.orus_zero_scale;
        let sc2x = fc2x as f32 * self.orus_zero_scale;
        let sc2y = fc2y as f32 * self.orus_zero_scale;
        let sex = fex as f32 * self.orus_zero_scale;
        let sey = fey as f32 * self.orus_zero_scale;

        if self.record_strokes {
            let (tsx, tsy) = self.transform_point(self.cur_x, self.cur_y);
            let (fsx, fsy) = self.apply_transform_flags_with_raw(self.cur_x, self.cur_y, tsx, tsy);
            self.strokes.push(PfrStroke {
                stroke_type: PfrStrokeType::Curve,
                start_x: fsx as f32 * self.orus_zero_scale,
                start_y: fsy as f32 * self.orus_zero_scale,
                end_x: sex,
                end_y: sey,
                control1_x: sc1x,
                control1_y: sc1y,
                control2_x: sc2x,
                control2_y: sc2y,
                width: self.current_stroke_width,
                is_horizontal: false,
                is_vertical: false,
            });
            self.stroke_has_pos = true;
            self.last_stroke_x = sex;
            self.last_stroke_y = sey;
            return;
        }

        if self.current_contour.commands.is_empty() {
            let (tpx, tpy) = self.transform_point(self.prev_x, self.prev_y);
            let (fpx, fpy) = self.apply_transform_flags_with_raw(self.prev_x, self.prev_y, tpx, tpy);
            self.current_contour.commands.push(PfrCmd::move_to(
                fpx as f32 * self.orus_zero_scale,
                fpy as f32 * self.orus_zero_scale,
            ));
        }

        self.current_contour.commands.push(PfrCmd::curve_to(
            sc1x, sc1y,
            sc2x, sc2y,
            sex, sey,
        ));
    }

    /// Add curve points (collinear case)
    fn add_curve_points(&mut self, x0: i16, y0: i16, x1: i16, y1: i16, x2: i16, y2: i16, x3: i16, y3: i16) {
        let max_jump: i32 = 50000;
        let jump_to_c1 = (x1 as i32 - x0 as i32).abs().max((y1 as i32 - y0 as i32).abs());
        let jump_to_c2 = (x2 as i32 - x0 as i32).abs().max((y2 as i32 - y0 as i32).abs());
        let jump_to_end = (x3 as i32 - x0 as i32).abs().max((y3 as i32 - y0 as i32).abs());
        if jump_to_c1 > max_jump || jump_to_c2 > max_jump || jump_to_end > max_jump {
            if !self.current_contour.commands.is_empty() {
                self.finish_contour();
            }
            self.cur_x = x3;
            self.cur_y = y3;
            self.prev_x = x2;
            self.prev_y = y2;
            return;
        }

        let (tx0, ty0) = self.transform_point(x0, y0);
        let (tx1, ty1) = self.transform_point(x1, y1);
        let (tx2, ty2) = self.transform_point(x2, y2);
        let (tx3, ty3) = self.transform_point(x3, y3);

        let (mut fx0, mut fy0) = self.apply_transform_flags_with_raw(x0, y0, tx0, ty0);
        let (mut fx1, mut fy1) = self.apply_transform_flags_with_raw(x1, y1, tx1, ty1);
        let (mut fx2, mut fy2) = self.apply_transform_flags_with_raw(x2, y2, tx2, ty2);
        let (mut fx3, mut fy3) = self.apply_transform_flags_with_raw(x3, y3, tx3, ty3);

        let sx0 = fx0 as f32 * self.orus_zero_scale;
        let sy0 = fy0 as f32 * self.orus_zero_scale;
        let sx1 = fx1 as f32 * self.orus_zero_scale;
        let sy1 = fy1 as f32 * self.orus_zero_scale;
        let sx2 = fx2 as f32 * self.orus_zero_scale;
        let sy2 = fy2 as f32 * self.orus_zero_scale;
        let sx3 = fx3 as f32 * self.orus_zero_scale;
        let sy3 = fy3 as f32 * self.orus_zero_scale;

        if self.record_strokes {
            if !self.stroke_has_pos {
                self.stroke_has_pos = true;
                self.last_stroke_x = sx3;
                self.last_stroke_y = sy3;
                self.prev_x = x2;
                self.prev_y = y2;
                return;
            }
            self.strokes.push(PfrStroke {
                stroke_type: PfrStrokeType::Curve,
                start_x: self.last_stroke_x,
                start_y: self.last_stroke_y,
                end_x: sx3,
                end_y: sy3,
                control1_x: sx1,
                control1_y: sy1,
                control2_x: sx2,
                control2_y: sy2,
                width: self.current_stroke_width,
                is_horizontal: false,
                is_vertical: false,
            });
            self.stroke_has_pos = true;
            self.last_stroke_x = sx3;
            self.last_stroke_y = sy3;
            self.prev_x = x2;
            self.prev_y = y2;
            return;
        }

        if self.current_contour.commands.is_empty() {
            self.current_contour.commands.push(PfrCmd::move_to(sx0, sy0));
        }
        self.current_contour
            .commands
            .push(PfrCmd::curve_to(sx1, sy1, sx2, sy2, sx3, sy3));

        self.prev_x = x2;
        self.prev_y = y2;
    }

    /// Parse compound glyph with full modulo-6 encoding
    fn parse_compound_glyph(&mut self) {
        const MAX_COMPOUND_DEPTH: u32 = 8;
        if self.depth >= MAX_COMPOUND_DEPTH {
            log(&format!(
                "  [compound] depth limit ({}) reached, skipping", MAX_COMPOUND_DEPTH
            ));
            return;
        }

        // Extract component count from header byte (bits 5-0)
        let component_count = (self.data[0] & 0x3F) as usize;

        log(&format!(
            "  [compound] {} components, gps_offset=0x{:X}, data[0]=0x{:02X}, data_len={}",
            component_count, self.glyph_gps_offset, self.data[0], self.data.len()
        ));

        // Check for extra data (bit 6 of header)
        if (self.data[0] & 0x40) != 0 && self.pos < self.data.len() {
            let extra_count = self.data[self.pos] as usize;
            self.pos += 1;
            for _ in 0..extra_count {
                if self.pos >= self.data.len() { break; }
                let len = self.data[self.pos] as usize;
                self.pos += 1;
                self.pos += len + 1; // Skip type byte + data
            }
        }

        // Two-pass approach
        // Initialize offset accumulator with this glyph's GPS offset
        let mut offset_accumulator: i32 = self.glyph_gps_offset;

        // PASS 1: Collect all component records
        struct CompRecord {
            x_scale: i32,
            y_scale: i32,
            x_offset: i32,
            y_offset: i32,
            glyph_offset: i32,
            subglyph_size: i32,
        }
        let mut records = Vec::new();

        for _ in 0..component_count {
            if self.pos >= self.data.len() { break; }

            let format_byte = self.data[self.pos] as i32;
            self.pos += 1;

            // Modulo-6 decoding
            let x_format = format_byte % 6;
            let y_format = (format_byte / 6) % 6;
            let offset_format = format_byte / 36;

            // Parse X transform
            let (x_scale, x_offset) = self.parse_transform_modulo6(x_format);
            // Parse Y transform
            let (y_scale, y_offset) = self.parse_transform_modulo6(y_format);
            // Parse glyph offset
            let (glyph_offset, subglyph_size) = self.parse_glyph_offset_modulo6(offset_format, &mut offset_accumulator);

            log(&format!(
                "    comp[{}]: fmt={} (x={},y={},off={}), scale=({},{}), offset=({},{}), glyph_off=0x{:X}, size={}",
                records.len(), format_byte, x_format, y_format, offset_format,
                x_scale, y_scale, x_offset, y_offset, glyph_offset, subglyph_size
            ));

            records.push(CompRecord {
                x_scale, y_scale, x_offset, y_offset, glyph_offset, subglyph_size,
            });
        }

        // Build sorted offset list and offset→size map (matching C# two-pass approach)
        let mut sorted_offsets: Vec<i32> = records.iter().map(|r| r.glyph_offset).collect();
        sorted_offsets.push(self.glyph_gps_offset); // Parent glyph as upper bound
        sorted_offsets.sort();
        sorted_offsets.dedup();

        // Build offset→size map from adjacent sorted offsets
        let mut offset_size_map = std::collections::HashMap::<i32, usize>::new();
        for i in 0..sorted_offsets.len().saturating_sub(1) {
            let size = (sorted_offsets[i + 1] - sorted_offsets[i]) as usize;
            offset_size_map.insert(sorted_offsets[i], size);
        }

        // Build extended known offsets for sub-parsers (matching C# knownGpsOffsets mutation)
        let mut extended_offsets: Vec<usize> = self.known_gps_offsets
            .map(|o| o.to_vec())
            .unwrap_or_default();
        for &off in &sorted_offsets {
            if off >= 0 {
                let off_usize = off as usize;
                if let Err(idx) = extended_offsets.binary_search(&off_usize) {
                    extended_offsets.insert(idx, off_usize);
                }
            }
        }

        // PASS 2: Parse each subglyph
        // glyph_offset is relative to GPS section start.
        for record in &records {
            let glyph_offset = record.glyph_offset;
            let (abs_pos, max_size) = match self.subglyph_bounds(glyph_offset) {
                Some(v) => v,
                None => continue,
            };

            let subglyph_size = if record.subglyph_size > 0 {
                (record.subglyph_size as usize).min(max_size)
            } else if let Some(&map_size) = offset_size_map.get(&glyph_offset) {
                map_size.min(max_size)
            } else {
                max_size.min(64) // Default limit
            };

            if subglyph_size == 0 { continue; }

            let subglyph_data = &self.gps_data[abs_pos..abs_pos + subglyph_size];

            // Reject suspicious subglyph headers
            if subglyph_data[0] == 0xFF || subglyph_data[0] == 0xFE {
                continue;
            }

            // Treat scale=0 as identity (4096)
            let comp_x_scale = if record.x_scale == 0 { 4096 } else { record.x_scale };
            let comp_y_scale = if record.y_scale == 0 { 4096 } else { record.y_scale };

            // Create sub-parser and parse recursively
            let font_matrix = [self.matrix_a, self.matrix_b, self.matrix_c, self.matrix_d];
            let mut sub_parser = Pfr1HeaderParser::new(
                subglyph_data,
                &font_matrix,
                self.outline_resolution,
                self.max_x_orus,
                self.max_y_orus,
                self.std_vw,
                self.std_hw,
                self.gps_data,
                self.gps_base,
                self.gps_len,
                glyph_offset,  // subglyph's GPS-section-relative offset
                Some(&extended_offsets),
                None,
                self.font_metrics.as_ref(),
                self.target_em_px,
            );

            sub_parser.recursion_depth = self.recursion_depth + 1;
            sub_parser.depth = self.depth + 1;
            sub_parser.diag_char_code = self.diag_char_code;
            if self.font_stroke_tables_available {
                sub_parser.font_stroke_x_count = self.font_stroke_x_count;
                sub_parser.font_stroke_y_count = self.font_stroke_y_count;
                sub_parser.font_stroke_x_keys = self.font_stroke_x_keys.clone();
                sub_parser.font_stroke_y_keys = self.font_stroke_y_keys.clone();
                sub_parser.font_stroke_x_scales = self.font_stroke_x_scales.clone();
                sub_parser.font_stroke_y_scales = self.font_stroke_y_scales.clone();
                sub_parser.font_stroke_x_values = self.font_stroke_x_values.clone();
                sub_parser.font_stroke_y_values = self.font_stroke_y_values.clone();
                sub_parser.font_stroke_tables_available = true;
            }

            // Inherit parent's coord state and apply component transform through matrix
            sub_parser.copy_grid_fit_from(self);
            sub_parser.copy_parent_coord_state(self);
            sub_parser.apply_component_transform(
                record.x_offset as i16, record.y_offset as i16,
                comp_x_scale, comp_y_scale,
            );

            let sub_glyph = sub_parser.parse();
            let sub_pts: usize = sub_glyph.contours.iter().map(|c| c.commands.len()).sum();
            let data_preview: Vec<String> = subglyph_data.iter().take(8).map(|b| format!("{:02X}", b)).collect();
            log(&format!(
                "    sub: abs_pos=0x{:X}, max_size={}, used_size={}, data=[{}], contours={}, pts={} fontScale=({},{})",
                abs_pos, max_size, subglyph_size, data_preview.join(" "),
                sub_glyph.contours.len(), sub_pts,
                sub_parser.font_scale_x, sub_parser.font_scale_y
            ));
            if !sub_glyph.contours.is_empty() {
                // Offset and scale already applied through the matrix by apply_component_transform.
                // Just copy contours directly.
                for contour in &sub_glyph.contours {
                    self.contours.push(contour.clone());
                }
            }
        }
        log(&format!(
            "  [compound] result: {} contours, {} total pts",
            self.contours.len(),
            self.contours.iter().map(|c| c.commands.len()).sum::<usize>()
        ));
    }

    /// Apply component transform (scale + offset) and merge into this glyph.
    /// Only used for identity-scale components; non-identity-scale components use
    /// the first-point correction path in parse_compound_glyph.
    fn merge_transformed_glyph(&mut self, source: &OutlineGlyph, x_offset: i16, y_offset: i16, x_scale: i16, y_scale: i16) {
        if source.contours.is_empty() {
            return;
        }

        let scale_x = x_scale as f32 / 4096.0;
        let scale_y = y_scale as f32 / 4096.0;

        // Offsets are in orus space; convert to pixel space using targetEmPx/outlineResolution.
        let pixel_scale = if self.outline_resolution > 0 && self.target_em_px > 0 {
            self.target_em_px as f32 / self.outline_resolution as f32
        } else {
            1.0
        };

        let x_sign = if self.x_transform_flag == 1 || self.x_transform_flag == 3 { -1.0 } else { 1.0 };
        let y_sign = if self.y_transform_flag == 1 || self.y_transform_flag == 3 { -1.0 } else { 1.0 };
        // Add rounding bias to merge offset (Phase 15): compound glyph decomposition splits
        // the coordinate mapping into sub-glyph transform + merge offset. Without rounding,
        // the merge offset truncates toward zero, losing up to ~1px (e.g. apostrophe
        // in Volter_700 compound glyph positioned 1px too low).
        let rounding = if self.coord_shift > 0 { 1i32 << (self.coord_shift - 1) } else { 0 };
        let x_off = if self.target_em_px > 0 && self.coord_shift > 0 {
            let raw = (x_offset as i32 * self.font_scale_x as i32 + rounding) >> self.coord_shift;
            raw as f32 * x_sign
        } else {
            x_offset as f32 * pixel_scale * x_sign
        };
        let y_off = if self.target_em_px > 0 && self.coord_shift > 0 {
            let raw = (y_offset as i32 * self.font_scale_y as i32 + rounding) >> self.coord_shift;
            raw as f32 * y_sign
        } else {
            y_offset as f32 * pixel_scale * y_sign
        };
        log(&format!(
            "    merge: offset=({},{}) pixel_scale={:.4} signs=({:.0},{:.0}) -> off=({:.1},{:.1})",
            x_offset, y_offset, pixel_scale, x_sign, y_sign, x_off, y_off
        ));

        for src_contour in &source.contours {
            let mut transformed = PfrContour::new();
            for cmd in &src_contour.commands {
                let mut tcmd = cmd.clone();
                if cmd.cmd_type != PfrCmdType::Close {
                    tcmd.x = cmd.x * scale_x + x_off;
                    tcmd.y = cmd.y * scale_y + y_off;
                    if cmd.cmd_type == PfrCmdType::CurveTo {
                        tcmd.x1 = cmd.x1 * scale_x + x_off;
                        tcmd.y1 = cmd.y1 * scale_y + y_off;
                        tcmd.x2 = cmd.x2 * scale_x + x_off;
                        tcmd.y2 = cmd.y2 * scale_y + y_off;
                    }
                }
                transformed.commands.push(tcmd);
            }
            self.contours.push(transformed);
        }
    }

    /// Compute absolute subglyph bounds given a GPS-relative glyph offset.
    /// Uses GPS section length + known offsets (if available) to cap sizes.
    fn subglyph_bounds(&self, glyph_offset: i32) -> Option<(usize, usize)> {
        let abs_pos = self.gps_base + glyph_offset;
        if abs_pos < 0 {
            return None;
        }
        let abs_pos = abs_pos as usize;
        if abs_pos >= self.gps_data.len() {
            return None;
        }

        let mut max_size = self.gps_data.len() - abs_pos;

        if glyph_offset >= 0 {
            let gps_off = glyph_offset as usize;
            if self.gps_len > 0 && gps_off < self.gps_len {
                max_size = max_size.min(self.gps_len - gps_off);
            }

            if let Some(offsets) = self.known_gps_offsets {
                let next = match offsets.binary_search(&gps_off) {
                    Ok(idx) => offsets.get(idx + 1).copied(),
                    Err(idx) => offsets.get(idx).copied(),
                };
                if let Some(next_off) = next {
                    if next_off > gps_off {
                        max_size = max_size.min(next_off - gps_off);
                    }
                }
            }
        }

        Some((abs_pos, max_size))
    }

    /// Parse transform (scale + offset) for one axis using modulo-6 format
    fn parse_transform_modulo6(&mut self, format: i32) -> (i32, i32) {
        let mut scale = 4096i32; // Default 1.0 in 1/4096 fixed-point
        let mut offset = 0i32;

        // Parse scale
        if format <= 2 {
            scale = 4096;
        } else if format == 5 {
            scale = 0;
        } else {
            // format 3 or 4: 2-byte scale
            if self.pos + 2 <= self.data.len() {
                scale = ((self.data[self.pos] as i32) << 8) | (self.data[self.pos + 1] as i32);
                self.pos += 2;
            }
        }

        // Parse offset
        if format == 0 || format == 5 {
            offset = 0;
        } else if format == 1 || format == 3 {
            // 1-byte signed offset
            if self.pos < self.data.len() {
                offset = self.data[self.pos] as i8 as i32;
                self.pos += 1;
            }
        } else {
            // format 2 or 4: 2-byte signed offset
            if self.pos + 2 <= self.data.len() {
                offset = i16::from_be_bytes([self.data[self.pos], self.data[self.pos + 1]]) as i32;
                self.pos += 2;
            }
        }

        (scale, offset)
    }

    /// Parse glyph offset using modulo-6 encoding
    fn parse_glyph_offset_modulo6(&mut self, format: i32, accumulator: &mut i32) -> (i32, i32) {
        let mut offset = 0i32;
        let mut subglyph_size = 0i32;

        match format {
            0 => {
                // 1-byte relative — delta = size
                if self.pos < self.data.len() {
                    let delta = self.data[self.pos] as i32;
                    self.pos += 1;
                    subglyph_size = delta;
                    *accumulator -= delta;
                    offset = *accumulator;
                }
            }
            1 => {
                // 1-byte + 256 relative — delta = size
                if self.pos < self.data.len() {
                    let delta = self.data[self.pos] as i32 + 256;
                    self.pos += 1;
                    subglyph_size = delta;
                    *accumulator -= delta;
                    offset = *accumulator;
                }
            }
            2 => {
                // 2-byte relative — delta = size
                if self.pos + 2 <= self.data.len() {
                    let delta = ((self.data[self.pos] as i32) << 8) | (self.data[self.pos + 1] as i32);
                    self.pos += 2;
                    subglyph_size = delta;
                    *accumulator -= delta;
                    offset = *accumulator;
                }
            }
            3 => {
                // 3-byte: RELATIVE offset, NO accumulator update
                if self.pos + 3 <= self.data.len() {
                    let combined = ((self.data[self.pos] as i32) << 16)
                        | ((self.data[self.pos + 1] as i32) << 8)
                        | (self.data[self.pos + 2] as i32);
                    self.pos += 3;
                    subglyph_size = combined >> 15;
                    let delta = combined & 0x7FFF;
                    offset = *accumulator - delta;
                }
            }
            4 => {
                // 3-byte: ABSOLUTE offset, NO accumulator update
                if self.pos + 3 <= self.data.len() {
                    let combined = ((self.data[self.pos] as i32) << 16)
                        | ((self.data[self.pos + 1] as i32) << 8)
                        | (self.data[self.pos + 2] as i32);
                    self.pos += 3;
                    subglyph_size = combined >> 15;
                    offset = combined & 0x7FFF;
                }
            }
            5 => {
                // 4-byte: ABSOLUTE 23-bit offset + 9-bit size
                if self.pos + 4 <= self.data.len() {
                    let combined = ((self.data[self.pos] as i32) << 24)
                        | ((self.data[self.pos + 1] as i32) << 16)
                        | ((self.data[self.pos + 2] as i32) << 8)
                        | (self.data[self.pos + 3] as i32);
                    self.pos += 4;
                    subglyph_size = (combined >> 23) & 0x1FF;
                    offset = combined & 0x7FFFFF;
                }
            }
            6 => {
                // 5-byte: 2-byte size + 3-byte absolute offset
                if self.pos + 5 <= self.data.len() {
                    subglyph_size = ((self.data[self.pos] as i32) << 8) | (self.data[self.pos + 1] as i32);
                    offset = ((self.data[self.pos + 2] as i32) << 16)
                        | ((self.data[self.pos + 3] as i32) << 8)
                        | (self.data[self.pos + 4] as i32);
                    self.pos += 5;
                }
            }
            _ => {
                // format 7+: use accumulator directly
                offset = *accumulator;
            }
        }

        (offset, subglyph_size)
    }

    fn finish_contour(&mut self) {
        if self.record_strokes {
            self.stroke_has_pos = false;
            return;
        }
        if self.current_contour.commands.len() > 1 {
            self.contours.push(self.current_contour.clone());
        }
        self.current_contour = PfrContour::new();
    }

    fn remove_duplicate_points(&mut self) {
        // In grid-fit pixel space, points half a pixel apart are real
        // geometry — only collapse true duplicates there.
        let eps = if self.grid_fit { 1.0 / 32.0 } else { 0.5 };
        for contour in &mut self.contours {
            let mut deduped = Vec::new();
            let mut last_x = f32::MIN;
            let mut last_y = f32::MIN;

            for cmd in &contour.commands {
                if cmd.cmd_type == PfrCmdType::Close {
                    deduped.push(cmd.clone());
                    continue;
                }
                if (cmd.x - last_x).abs() > eps || (cmd.y - last_y).abs() > eps {
                    deduped.push(cmd.clone());
                    last_x = cmd.x;
                    last_y = cmd.y;
                }
            }
            contour.commands = deduped;
        }
    }

    fn trim_contour_outliers(&mut self) {
        for contour in &mut self.contours {
            while contour.commands.len() >= 4 {
                let n = contour.commands.len();
                let last = &contour.commands[n - 1];

                let mut min_x = f32::MAX;
                let mut max_x = f32::MIN;
                for i in 0..n - 1 {
                    let cmd = &contour.commands[i];
                    if cmd.x < min_x { min_x = cmd.x; }
                    if cmd.x > max_x { max_x = cmd.x; }
                }
                let box_width = max_x - min_x;
                if box_width < 1.0 { break; }

                let ext_left = (min_x - last.x).max(0.0);
                let ext_right = (last.x - max_x).max(0.0);
                let extension = ext_left.max(ext_right);

                if extension > box_width * 0.5 {
                    contour.commands.pop();
                } else {
                    break;
                }
            }
        }
    }

    fn close_contours(&mut self) {
        let eps = if self.grid_fit { 1.0 / 32.0 } else { 0.5 };
        for contour in &mut self.contours {
            if contour.commands.is_empty() { continue; }

            let first = &contour.commands[0];
            if first.cmd_type != PfrCmdType::MoveTo { continue; }

            let last = contour.commands.last().unwrap();
            if last.cmd_type == PfrCmdType::Close { continue; }

            let start_x = first.x;
            let start_y = first.y;
            let end_x = last.x;
            let end_y = last.y;

            if (end_x - start_x).abs() > eps || (end_y - start_y).abs() > eps {
                contour.commands.push(PfrCmd::line_to(start_x, start_y));
            }
        }
    }
}

// ========== PFR1 Direct Parser (fallback) ==========

/// PFR1DirectParser - Simple byte-command parser (fallback)
pub struct Pfr1DirectParser<'a> {
    data: &'a [u8],
}

impl<'a> Pfr1DirectParser<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }

    pub fn parse(&self) -> OutlineGlyph {
        let mut glyph = OutlineGlyph::new();
        let mut contours: Vec<PfrContour> = Vec::new();
        let mut current_contour = PfrContour::new();
        let mut pos = 0usize;
        let mut cur_x: f32 = 0.0;
        let mut cur_y: f32 = 0.0;

        let mut finish_contour = |contours: &mut Vec<PfrContour>, current: &mut PfrContour| {
            if !current.commands.is_empty() {
                contours.push(current.clone());
                current.commands.clear();
            }
        };

        let mut read_coord_pair = |data: &[u8], pos: &mut usize| -> (f32, f32) {
            let mut x = 0.0f32;
            let mut y = 0.0f32;
            if *pos < data.len() {
                x = data[*pos] as i8 as f32;
                *pos += 1;
            }
            if *pos < data.len() {
                y = data[*pos] as i8 as f32;
                *pos += 1;
            }
            (x, y)
        };

        while pos < self.data.len() {
            let cmd = self.data[pos];
            pos += 1;

            // End of glyph markers
            if cmd == 0x00 || cmd >= 0xE0 {
                finish_contour(&mut contours, &mut current_contour);
                break;
            }

            let cmd_type = (cmd >> 4) & 0x0F;
            let flags = cmd & 0x0F;

            match cmd_type {
                0 => {
                    if cmd == 0x01 || cmd == 0x05 {
                        let (dx, dy) = read_coord_pair(self.data, &mut pos);
                        cur_x += dx;
                        cur_y += dy;
                        finish_contour(&mut contours, &mut current_contour);
                        current_contour.commands.push(PfrCmd::move_to(cur_x, cur_y));
                    }
                }
                1 => {
                    let (dx, dy) = read_coord_pair(self.data, &mut pos);
                    cur_x += dx;
                    cur_y += dy;
                    current_contour.commands.push(PfrCmd::line_to(cur_x, cur_y));
                }
                2 => {
                    let (x, y) = read_coord_pair(self.data, &mut pos);
                    if (flags & 0x08) != 0 {
                        cur_x = x;
                        cur_y = y;
                    } else {
                        cur_x += x;
                        cur_y += y;
                    }
                    finish_contour(&mut contours, &mut current_contour);
                    current_contour.commands.push(PfrCmd::move_to(cur_x, cur_y));
                }
                _ => {
                    if pos < self.data.len().saturating_sub(1) {
                        pos += 2;
                    } else {
                        break;
                    }
                }
            }
        }

        finish_contour(&mut contours, &mut current_contour);
        glyph.contours = contours;
        glyph
    }
}

// ========== Bitmap Glyph Parser ==========

/// Parse a bitmap glyph from GPS data
pub fn parse_bitmap_glyph(data: &[u8], char_code: u32) -> Option<BitmapGlyph> {
    if data.is_empty() {
        return None;
    }

    let mut pos = 0;
    let format_byte = data[pos];
    pos += 1;

    let image_format = (format_byte >> 6) & 0x03;
    let escapement_format = (format_byte >> 4) & 0x03;
    let size_format = (format_byte >> 2) & 0x03;
    let position_format = format_byte & 0x03;

    fn read_u_n(data: &[u8], pos: &mut usize, n: usize) -> u32 {
        let mut v = 0u32;
        for _ in 0..n {
            if *pos >= data.len() {
                break;
            }
            v = (v << 8) | data[*pos] as u32;
            *pos += 1;
        }
        v
    }

    fn read_i_n(data: &[u8], pos: &mut usize, n: usize) -> i32 {
        let v = read_u_n(data, pos, n);
        if n == 0 {
            return 0;
        }
        let sign_bit = 1u32 << (n * 8 - 1);
        if v & sign_bit != 0 {
            let mask = (!0u32) << (n * 8);
            (v | mask) as i32
        } else {
            v as i32
        }
    }

    // PFR bitmap glyph header. Per-field widths follow the spec (and match the
    // verified Fontinator/Director reference): position/size format 0 packs the
    // two values into one byte's nibbles; format 1 is one byte each (signed for
    // position); format 2/3 widen to 2/3 bytes.
    let read_signed_nibbles = |data: &[u8], pos: &mut usize| -> (i32, i32) {
        if *pos >= data.len() {
            return (0, 0);
        }
        let b = data[*pos];
        *pos += 1;
        let mut a = ((b >> 4) & 0x0F) as i32;
        let mut c = (b & 0x0F) as i32;
        if a & 0x8 != 0 { a -= 16; }
        if c & 0x8 != 0 { c -= 16; }
        (a, c)
    };

    let (x_pos, y_pos) = match position_format {
        0 => read_signed_nibbles(data, &mut pos),
        1 => (read_i_n(data, &mut pos, 1), read_i_n(data, &mut pos, 1)),
        2 => (read_i_n(data, &mut pos, 2), read_i_n(data, &mut pos, 2)),
        _ => (read_i_n(data, &mut pos, 3), read_i_n(data, &mut pos, 3)),
    };

    let (x_size, y_size) = match size_format {
        0 => (0u32, 0u32),
        1 => {
            if pos >= data.len() { (0, 0) } else {
                let b = data[pos];
                pos += 1;
                (((b >> 4) & 0x0F) as u32, (b & 0x0F) as u32)
            }
        }
        2 => (read_u_n(data, &mut pos, 1), read_u_n(data, &mut pos, 1)),
        _ => (read_u_n(data, &mut pos, 2), read_u_n(data, &mut pos, 2)),
    };

    let set_width = match escapement_format {
        0 => 0u32,
        1 => read_i_n(data, &mut pos, 1).max(0) as u32,
        2 => read_i_n(data, &mut pos, 2).max(0) as u32,
        _ => read_i_n(data, &mut pos, 3).max(0) as u32,
    };

    let x_size = x_size.min(u16::MAX as u32) as u16;
    let y_size = y_size.min(u16::MAX as u32) as u16;
    let set_width = set_width.min(u16::MAX as u32) as u16;
    let x_pos = x_pos.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    let y_pos = y_pos.clamp(i16::MIN as i32, i16::MAX as i32) as i16;

    if x_size == 0 || y_size == 0 {
        return None;
    }

    let total_bits = (x_size as usize).saturating_mul(y_size as usize);
    if total_bits == 0 {
        return None;
    }

    let remaining = &data[pos..];

    let image_data = match image_format {
        0 => {
            // Packed bits (imageFormat=0), no row padding
            let expected = (total_bits + 7) / 8;
            if expected > remaining.len() || total_bits > 1_000_000 {
                return None;
            }
            remaining[..expected.min(remaining.len())].to_vec()
        }
        1 => {
            // 4-bit RLE (imageFormat=1)
            if total_bits > 1_000_000 {
                return None;
            }
            // Heuristic: reject if RLE stream is far too small for the declared size
            if total_bits > remaining.len() * 256 {
                return None;
            }
            decode_rle_bitmap(remaining, x_size, y_size)
        }
        _ => {
            remaining.to_vec()
        }
    };

    Some(BitmapGlyph {
        char_code,
        image_format,
        x_pos,
        y_pos,
        x_size,
        y_size,
        set_width,
        image_data,
    })
}

/// Decode 4-bit RLE bitmap
fn decode_rle_bitmap(data: &[u8], width: u16, height: u16) -> Vec<u8> {
    // PFR1 bitmap image_format 1: each NIBBLE (high then low per byte) is a run
    // length, and runs ALTERNATE background (unset) / foreground (set), starting
    // with background. e.g. the bold Volter `l` is `[0x0E]` → run 0 unset, run 14
    // set → a solid 2×7 bar. The earlier decoder treated `byte>>4` as a count and
    // `byte&0xF` as a color, reading `0x0E` as "0 cells of color 14" → an empty
    // glyph (the `caLLed` bug). Output is contiguous packed bits (row-major),
    // matching the rasterizer's `bit_index = row*width + col` read.
    let total_bits = width as usize * height as usize;
    let total_bytes = (total_bits + 7) / 8;
    let mut result = vec![0u8; total_bytes];
    let mut out_pos = 0usize;
    let mut set = false; // first run is background

    'outer: for &byte in data {
        for nib in [((byte >> 4) & 0x0F) as usize, (byte & 0x0F) as usize] {
            if set {
                for _ in 0..nib {
                    if out_pos >= total_bits {
                        break 'outer;
                    }
                    result[out_pos / 8] |= 1 << (7 - (out_pos % 8));
                    out_pos += 1;
                }
            } else {
                out_pos += nib;
                if out_pos >= total_bits {
                    break 'outer;
                }
            }
            set = !set;
        }
    }

    result
}

// ========== Glyph Scoring ==========

/// Score a parsed glyph to determine quality
pub fn score_glyph(glyph: &OutlineGlyph, is_header_parser: bool) -> i32 {
    let mut score: i32 = 0;

    // Points per contour
    score += glyph.contours.len() as i32 * 10;

    // Points per point
    let total_points: usize = glyph.contours.iter()
        .map(|c| c.commands.len())
        .sum();
    score += total_points as i32;

    // Curve bonus
    let curve_count: usize = glyph.contours.iter()
        .flat_map(|c| &c.commands)
        .filter(|cmd| cmd.cmd_type == PfrCmdType::CurveTo)
        .count();
    score += curve_count as i32 * 5;

    // PFR1HeaderParser bonus
    if is_header_parser {
        score += 300;
    }

    // Coordinate range bonus
    let mut min_x = f32::MAX;
    let mut max_x = f32::MIN;
    let mut min_y = f32::MAX;
    let mut max_y = f32::MIN;
    for contour in &glyph.contours {
        for cmd in &contour.commands {
            min_x = min_x.min(cmd.x);
            max_x = max_x.max(cmd.x);
            min_y = min_y.min(cmd.y);
            max_y = max_y.max(cmd.y);
        }
    }
    let x_range = (max_x - min_x) as i32;
    let y_range = (max_y - min_y) as i32;
    if x_range > 10 { score += 20; }
    if y_range > 10 { score += 20; }

    score
}

// ========== Utility ==========

fn calculate_encoding_14(b: u32) -> u32 {
    let v = CURVE_TABLE_14B[((b >> 3) & 3) as usize] + 16 * CURVE_TABLE_14A[((b >> 5) & 7) as usize];
    let v2 = v * 16;
    CURVE_TABLE_14A[(b & 7) as usize] + v2
}

/// Parse a single glyph from GPS data using all parsers, pick best result
pub fn parse_glyph(
    gps_data: &[u8],
    full_data: &[u8],
    char_record: &CharacterRecord,
    font_matrix: &[i32; 4],
    outline_resolution: u16,
    max_x_orus: u8,
    max_y_orus: u8,
    std_vw: f32,
    std_hw: f32,
    font_metrics: Option<&FontMetrics>,
    target_em_px: i32,
    gps_section_offset: usize,
    gps_section_size: usize,
    known_gps_offsets: &[usize],
    physical_font: Option<&PhysicalFontRecord>,
    grid_fit: bool,
) -> Option<OutlineGlyph> {
    let start = char_record.gps_offset as usize;
    let size = char_record.gps_size as usize;
    let char_code = char_record.char_code;
    let ch = if char_code >= 32 && char_code < 127 { char_code as u8 as char } else { '?' };

    // Handle glyphs with no GPS data (gpsSize <= 1)
    // These are characters with no outline - spaces, control chars, null, etc.
    // They still need to be in the glyph map for text layout (set_width matters)
    if size <= 1 {
        let mut glyph = OutlineGlyph::new();
        glyph.char_code = char_code;
        glyph.set_width = char_record.set_width as f32;
        return Some(glyph);
    }

    if start + size > gps_data.len() {
        log(&format!("    SKIP char {} ('{}') - gps out of range (start={}, size={}, gps_len={})",
            char_code, ch, start, size, gps_data.len()));
        return None;
    }

    let glyph_data = &gps_data[start..start + size];

    // Detect compound glyph for diagnostic logging
    let is_compound_detected = size > 0 && (glyph_data[0] >> 6) & 3 >= 2 && (glyph_data[0] & 0x3F) > 0;

    // Try PFR1HeaderParser first (primary, +300 score bonus)
    let mut best_glyph: Option<OutlineGlyph> = None;
    let mut best_score: i32 = -1;
    let mut best_is_header = false;
    let mut header_score: i32 = 0;
    let mut direct_score: i32 = 0;
    let mut header_contours = 0usize;
    let mut direct_contours = 0usize;

    {
        let known_offsets = if known_gps_offsets.is_empty() { None } else { Some(known_gps_offsets) };

        let mut parser = Pfr1HeaderParser::new(
            glyph_data,
            font_matrix,
            outline_resolution,
            max_x_orus,
            max_y_orus,
            std_vw,
            std_hw,
            full_data,
            gps_section_offset as i32,
            gps_section_size,
            start as i32,  // glyph's GPS-section-relative offset (for compound glyph offset accumulator)
            known_offsets,
            physical_font,
            font_metrics,
            target_em_px,
        );
        parser.diag_char_code = char_code;
        if grid_fit {
            parser.enable_grid_fit(physical_font);
            // Set width converted metrics -> outline units, the band the
            // re-centering correction scans (the advance formula's 16.16 em
            // fraction round-tripped through the outline resolution).
            let metrics_res = physical_font
                .map(|p| p.metrics_resolution)
                .filter(|&r| r > 0)
                .unwrap_or(outline_resolution.max(1)) as i64;
            let frac = ((metrics_res >> 1) + ((char_record.set_width as i64) << 16)) / metrics_res;
            parser.fit_set_width_orus =
                ((frac * outline_resolution as i64 + 0x8000) >> 16) as i32;
        }
        let glyph = parser.parse();
        header_contours = glyph.contours.len();
        header_score = score_glyph(&glyph, true);

        if header_score > best_score && !glyph.contours.is_empty() {
            best_score = header_score;
            best_is_header = true;
            best_glyph = Some(glyph);
        }
    }

    // Try PFR1DirectParser as fallback. The grid-fit path skips it: its
    // output is orus-space and would win the score contest against small
    // pixel-space coordinates.
    if !grid_fit {
        let parser = Pfr1DirectParser::new(glyph_data);
        let glyph = parser.parse();
        direct_contours = glyph.contours.len();
        direct_score = score_glyph(&glyph, false);

        if direct_score > best_score && !glyph.contours.is_empty() {
            best_score = direct_score;
            best_is_header = false;
            best_glyph = Some(glyph);
        }
    }

    // Diagnostic logging for compound glyphs
    if is_compound_detected {
        log(&format!(
            "  [compound] char {} ('{}') winner={} hdr_contours={} dir_contours={} hdr_score={} dir_score={}",
            char_code, ch,
            if best_is_header { "header" } else { "direct" },
            header_contours, direct_contours, header_score, direct_score
        ));
    }

    // If neither parser produced contours, still return the glyph with set_width
    // (it may be a legitimately empty glyph like a narrow space variant)
    if best_glyph.is_none() {
        if size > 1 {
            log(&format!("    FAIL char {} ('{}') - no contours from either parser (hdr={}, dir={}, data[0]=0x{:02X}, size={})",
                char_code, ch, header_contours, direct_contours, glyph_data[0], size));
        }
        // Still create an empty glyph so the character exists in the font map
        let mut glyph = OutlineGlyph::new();
        glyph.char_code = char_code;
        glyph.set_width = char_record.set_width as f32;
        return Some(glyph);
    }

    if let Some(ref mut glyph) = best_glyph {
        glyph.char_code = char_code;
        glyph.set_width = char_record.set_width as f32;

        // Clamp and scale extreme coordinates. Grid-fit glyphs are in pixel
        // space, so the sanity band scales with the target size, not the em.
        let outline_res = if grid_fit && target_em_px > 0 {
            target_em_px as f32
        } else {
            outline_resolution as f32
        };
        let max_valid = outline_res * 3.0;
        let mut min_x = f32::MAX;
        let mut max_x = f32::MIN;
        let mut min_y = f32::MAX;
        let mut max_y = f32::MIN;
        for contour in &glyph.contours {
            for cmd in &contour.commands {
                min_x = min_x.min(cmd.x);
                max_x = max_x.max(cmd.x);
                min_y = min_y.min(cmd.y);
                max_y = max_y.max(cmd.y);
            }
        }

        for contour in &mut glyph.contours {
            for cmd in &mut contour.commands {
                cmd.x = cmd.x.clamp(-max_valid, max_valid);
                cmd.y = cmd.y.clamp(-max_valid, max_valid);
                cmd.x1 = cmd.x1.clamp(-max_valid, max_valid);
                cmd.y1 = cmd.y1.clamp(-max_valid, max_valid);
                cmd.x2 = cmd.x2.clamp(-max_valid, max_valid);
                cmd.y2 = cmd.y2.clamp(-max_valid, max_valid);
            }
        }

        let max_coord = max_x
            .abs()
            .max(max_y.abs())
            .max(min_x.abs())
            .max(min_y.abs());
        // Grid-fit glyphs are already snapped to the pixel grid; rescaling
        // would destroy the fit (Volter's tall accents legitimately exceed
        // 2x the em). Junk glyphs were clamped above and stay clamped.
        if !grid_fit && max_coord > outline_res * 2.0 && max_coord > 0.0 {
            log(&format!(
                "  [NORM] char {} ('{}') max_coord={:.1} outline_res={:.1} scale={:.4} bbox=({:.1},{:.1})..({:.1},{:.1}) compound={}",
                char_code, ch, max_coord, outline_res, outline_res / max_coord,
                min_x, min_y, max_x, max_y, is_compound_detected
            ));
            let scale = outline_res / max_coord;
            for contour in &mut glyph.contours {
                for cmd in &mut contour.commands {
                    cmd.x *= scale;
                    cmd.y *= scale;
                    cmd.x1 *= scale;
                    cmd.y1 *= scale;
                    cmd.x2 *= scale;
                    cmd.y2 *= scale;
                }
            }
        }
    }

    best_glyph
}
