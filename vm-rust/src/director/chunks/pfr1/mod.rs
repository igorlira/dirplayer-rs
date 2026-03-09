/// PFR1 Font Parser Module
///
/// Parses PFR1 (Portable Font Resource) fonts from XMED chunks,
/// producing outline glyphs with proportional widths that are
/// rasterized to bitmaps for the BitmapFont system.

pub mod bit_reader;
pub mod types;
pub mod header;
pub mod physical;
pub mod glyph;
pub mod rasterizer;
pub mod stroke_builder;

use log::debug;
use types::*;

const PFR1_VERBOSE_LOGS: bool = false;

fn log(msg: &str) {
    if PFR1_VERBOSE_LOGS {
        debug!("{}", msg);
    }
}

/// Main entry point: parse a PFR1 font from raw XMED chunk data.
/// Uses target_em_px=0 to keep coordinates in orus (outline resolution units) space.
/// Callers that know the target rendering size should use parse_pfr1_font_with_target()
/// instead, so zone tables produce correct piecewise-linear interpolation for that size.
pub fn parse_pfr1_font(data: &[u8]) -> Result<Pfr1ParsedFont, String> {
    parse_pfr1_font_with_target(data, 0)
}

/// Parse a PFR1 font with a target em size (in pixels) for header parser scaling.
pub fn parse_pfr1_font_with_target(data: &[u8], target_em_px: i32) -> Result<Pfr1ParsedFont, String> {
    log(&format!("PFR1 parser: parsing {} bytes", data.len()));

    let mut font = Pfr1ParsedFont::new();
    font.target_em_px = target_em_px;

    // 1. Parse PFR header
    let pfr_header = header::parse_pfr_header(data)?;
    font.header = pfr_header.clone();
    font.is_pfr1 = pfr_header.version == 1;
    font.gps_section_offset = pfr_header.gps_section_offset;
    font.pfr_black_pixel = pfr_header.pfr_black_pixel;

    log(&format!("  Header: version={}, physOffset=0x{:X}, gpsOffset=0x{:X}, gpsSize={}",
        pfr_header.version,
        pfr_header.phys_font_section_offset,
        pfr_header.gps_section_offset,
        pfr_header.gps_section_size));

    // 2. Parse logical font directory
    font.logical_fonts = header::parse_logical_font_directory(data, &pfr_header)?;
    if !font.logical_fonts.is_empty() {
        let m = &font.logical_fonts[0].font_matrix;
        log(&format!("  LogFont: matrix=[{}, {}, {}, {}], physSize={}, physOffset=0x{:X}",
            m[0], m[1], m[2], m[3],
            font.logical_fonts[0].size, font.logical_fonts[0].offset));
    } else {
        log("  LogFont: none (using identity matrix)");
    }

    // 3. Parse physical font section
    let phys_offset = pfr_header.phys_font_section_offset as usize;
    let mut phys_end = data.len();
    let phys_end_from_size = phys_offset.saturating_add(pfr_header.phys_font_section_size as usize);
    if phys_end_from_size > phys_offset && phys_end_from_size <= data.len() {
        phys_end = phys_end.min(phys_end_from_size);
    }
    let gps_offset = pfr_header.gps_section_offset as usize;
    if gps_offset > phys_offset && gps_offset <= data.len() {
        phys_end = phys_end.min(gps_offset);
    }

    font.physical_font = physical::parse_physical_font(data, phys_offset, phys_end, pfr_header.max_chars)?;
    // Header carries max orus values used by PFR1 glyph parsing
    font.physical_font.max_x_orus = pfr_header.max_x_orus;
    font.physical_font.max_y_orus = pfr_header.max_y_orus;
    // Initialize font-level stroke tables
    physical::initialize_stroke_tables_fallback(&mut font.physical_font);

    // Extract font name from physical font ID
    if !font.physical_font.font_id.is_empty() {
        font.font_name = font.physical_font.font_id.clone();
    } else {
        font.font_name = extract_font_name(data).unwrap_or_else(|| "PFR1_Font".to_string());
    }

    let debug_v_font = font.font_name.eq_ignore_ascii_case("v")
        || font.font_name.eq_ignore_ascii_case("volter_400_000")
        || font.font_name.to_lowercase().contains("volter");
    let debug_reaction = font.font_name.to_lowercase().contains("reaction");

    if debug_v_font {
        log(&format!(
            "  [debug] PFR1 font='{}' pfr_black_pixel={} gpsOffset=0x{:X} gpsSize={}",
            font.font_name,
            font.pfr_black_pixel,
            font.gps_section_offset,
            font.header.gps_section_size
        ));
    }

    log(&format!("  PhysFont: name='{}', outlineRes={}, bbox=({},{})..({},{}), {} chars, StdVW={}, StdHW={}",
        font.font_name,
        font.physical_font.outline_resolution,
        font.physical_font.x_min, font.physical_font.y_min,
        font.physical_font.x_max, font.physical_font.y_max,
        font.physical_font.char_records.len(),
        font.physical_font.metrics.std_vw,
        font.physical_font.metrics.std_hw));

    // Log first few char records for debugging
    for (i, cr) in font.physical_font.char_records.iter().enumerate().take(5) {
        let ch = if cr.char_code >= 32 && cr.char_code < 127 { cr.char_code as u8 as char } else { '?' };
        log(&format!("    Char[{}]: code={} ('{}'), width={}, gpsSize={}, gpsOff=0x{:X}",
            i, cr.char_code, ch, cr.set_width, cr.gps_size, cr.gps_offset));
    }

    // 4. Parse GPS section - parse each glyph
    let gps_offset = pfr_header.gps_section_offset as usize;
    let gps_size = pfr_header.gps_section_size as usize;

    if gps_offset + gps_size <= data.len() {
        let gps_data = &data[gps_offset..gps_offset + gps_size];

        let font_matrix = if !font.logical_fonts.is_empty() {
            font.logical_fonts[0].font_matrix
        } else {
            [256, 0, 0, 256]
        };

        let std_vw = font.physical_font.metrics.std_vw as f32;
        let std_hw = font.physical_font.metrics.std_hw as f32;
        let font_metrics = Some(&font.physical_font.metrics);

        let mut parsed_count = 0;
        let mut with_contours = 0;
        let mut bitmap_count = 0;
        let total_chars = font.physical_font.char_records.len();

        // Known GPS offsets for compound glyph subglyph size limiting
        let mut known_gps_offsets: Vec<usize> = font.physical_font.char_records
            .iter()
            .map(|cr| cr.gps_offset as usize)
            .collect();
        known_gps_offsets.sort_unstable();
        known_gps_offsets.dedup();

        for i in 0..total_chars {
            let char_record = font.physical_font.char_records[i].clone();
            let char_code = char_record.char_code;
            let char_byte = char_code as u8;

            let start = char_record.gps_offset as usize;
            let size = char_record.gps_size as usize;
            let debug_this = debug_v_font && (
                i < 5
                || (char_code >= 48 && char_code <= 57)
                || char_code == 70  // 'F'
                || char_code == 80  // 'P'
                || char_code == 119 // 'w'
            );

            // Empty glyphs (space / control) still need width for layout
            if size <= 1 {
                let mut glyph = OutlineGlyph::new();
                glyph.char_code = char_code;
                glyph.set_width = char_record.set_width as f32;
                if char_code <= 0xFF {
                    font.glyphs.insert(char_byte, glyph);
                    parsed_count += 1;
                }
                if debug_this {
                    let ch = if char_code >= 32 && char_code < 127 { char_code as u8 as char } else { '?' };
                    log(&format!(
                        "    Char {} ('{}'): empty glyph (gpsSize={}) setWidth={}",
                        char_code, ch, size, char_record.set_width
                    ));
                }
                continue;
            }

            if start + size > gps_data.len() {
                if debug_this {
                    let ch = if char_code >= 32 && char_code < 127 { char_code as u8 as char } else { '?' };
                    log(&format!(
                        "    Char {} ('{}'): gps out of range start={} size={} gps_len={}",
                        char_code, ch, start, size, gps_data.len()
                    ));
                }
                continue;
            }

            let glyph_data = &gps_data[start..start + size];
            let zeros_field = (glyph_data[0] >> 4) & 0x07;

            if zeros_field != 0 && font.physical_font.has_bitmap_section {
                // Bitmap glyph
                if debug_this {
                    let ch = if char_code >= 32 && char_code < 127 { char_code as u8 as char } else { '?' };
                    let hex: String = glyph_data.iter().take(16).map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                    let fmt = glyph_data[0];
                    let image_format = (fmt >> 6) & 0x03;
                    let escapement_format = (fmt >> 4) & 0x03;
                    let size_format = (fmt >> 2) & 0x03;
                    let position_format = fmt & 0x03;
                    log(&format!(
                        "    Char {} ('{}'): bitmap header fmt=0x{:02X} img={} esc={} size={} pos={} bytes=[{}]",
                        char_code, ch, fmt, image_format, escapement_format, size_format, position_format, hex
                    ));
                }
                if let Some(mut bmp) = glyph::parse_bitmap_glyph(glyph_data, char_code) {
                    if char_record.set_width > 0 {
                        bmp.set_width = char_record.set_width;
                    }
                    // Skip bitmap glyph if its pixel size doesn't match target em size
                    // (bitmap is for a different point size than requested)
                    let target_h = target_em_px.max(1) as u16;
                    let bmp_h = bmp.y_size;
                    if target_h > 0 && (bmp_h > target_h * 2 || bmp_h < target_h / 2) {
                        if debug_this {
                            let ch = if char_code >= 32 && char_code < 127 { char_code as u8 as char } else { '?' };
                            log(&format!(
                                "    Char {} ('{}'): SKIPPING bitmap glyph (size mismatch: bmp_h={} target_h={}), falling through to outline",
                                char_code, ch, bmp_h, target_h
                            ));
                        }
                        // Fall through to outline parsing below
                    } else {
                        if char_code <= 0xFF {
                            font.bitmap_glyphs.insert(char_byte, bmp.clone());
                            bitmap_count += 1;
                        }
                        if debug_this {
                            let ch = if char_code >= 32 && char_code < 127 { char_code as u8 as char } else { '?' };
                            log(&format!(
                                "    Char {} ('{}'): bitmap glyph (zeros=0x{:X}) size={} img={}x{} pos=({}, {})",
                                char_code,
                                ch,
                                zeros_field,
                                size,
                                bmp.x_size,
                                bmp.y_size,
                                bmp.x_pos,
                                bmp.y_pos
                            ));
                        }
                        // Don't skip outline parsing - outline rendering produces
                        // cleaner strokes than bitmap glyphs at small sizes.
                    }
                }
                // If bitmap parsing failed, fall through to outline parsing.
            }

            // Diagnostic: dump glyph header info for FFF Reaction font
            if debug_reaction && char_code >= 32 && char_code < 127 {
                let ch = char_code as u8 as char;
                let b0 = glyph_data[0];
                let outline_fmt = (b0 >> 6) & 3;
                let size_enc = (b0 >> 4) & 3;
                let orus_enc = (b0 >> 2) & 3;
                let count_enc = b0 & 3;
                let has_extra = (b0 & 0x08) != 0;
                let hex: String = glyph_data.iter().take(12).map(|b| format!("{:02X}", b)).collect::<Vec<_>>().join(" ");
                log(&format!(
                    "  [DIAG] Char {} ('{}') byte0=0x{:02X} outFmt={} sizeEnc={} orusEnc={} countEnc={} extraItems={} gpsSize={} bytes=[{}]",
                    char_code, ch, b0, outline_fmt, size_enc, orus_enc, count_enc, has_extra, size, hex
                ));
            }

            if let Some(outline_glyph) = glyph::parse_glyph(
                gps_data,
                data,
                &char_record,
                &font_matrix,
                font.physical_font.outline_resolution,
                font.physical_font.max_x_orus,
                font.physical_font.max_y_orus,
                std_vw,
                std_hw,
                font_metrics,
                target_em_px,
                gps_offset,
                gps_size,
                &known_gps_offsets,
                Some(&font.physical_font),
            ) {
                if !outline_glyph.contours.is_empty() {
                    with_contours += 1;
                }

                // Diagnostic: dump parsed contour info for FFF Reaction font
                if debug_reaction && char_code >= 32 && char_code < 127 {
                    let ch = char_code as u8 as char;
                    let n_contours = outline_glyph.contours.len();
                    let n_pts: usize = outline_glyph.contours.iter().map(|c| c.commands.len()).sum();
                    if n_contours > 0 {
                        let mut min_x = f32::MAX;
                        let mut min_y = f32::MAX;
                        let mut max_x = f32::MIN;
                        let mut max_y = f32::MIN;
                        for c in &outline_glyph.contours {
                            for cmd in &c.commands {
                                min_x = min_x.min(cmd.x);
                                min_y = min_y.min(cmd.y);
                                max_x = max_x.max(cmd.x);
                                max_y = max_y.max(cmd.y);
                            }
                        }
                        log(&format!(
                            "         -> {} contours, {} pts, bbox=({:.1},{:.1})..({:.1},{:.1}) set_w={:.1}",
                            n_contours, n_pts, min_x, min_y, max_x, max_y, outline_glyph.set_width
                        ));
                    } else {
                        log(&format!(
                            "         -> EMPTY (0 contours) set_w={:.1}",
                            outline_glyph.set_width
                        ));
                    }
                }
                if char_code <= 0xFF {
                    font.glyphs.insert(char_byte, outline_glyph);
                    parsed_count += 1;
                }
                if debug_this {
                    let ch = if char_code >= 32 && char_code < 127 { char_code as u8 as char } else { '?' };
                    let mut min_x = f32::MAX;
                    let mut max_x = f32::MIN;
                    let mut min_y = f32::MAX;
                    let mut max_y = f32::MIN;
                    let mut pts = 0usize;
                    if let Some(glyph) = font.glyphs.get(&char_byte) {
                        for contour in &glyph.contours {
                            for cmd in &contour.commands {
                                min_x = min_x.min(cmd.x);
                                max_x = max_x.max(cmd.x);
                                min_y = min_y.min(cmd.y);
                                max_y = max_y.max(cmd.y);
                                pts += 1;
                            }
                        }
                        if pts == 0 {
                            log(&format!(
                                "    Char {} ('{}'): outline EMPTY (size={} zeros=0x{:X})",
                                char_code, ch, size, zeros_field
                            ));
                        } else {
                            log(&format!(
                                "    Char {} ('{}'): outline pts={} contours={} bbox=({:.1},{:.1})..({:.1},{:.1})",
                                char_code,
                                ch,
                                pts,
                                glyph.contours.len(),
                                min_x,
                                min_y,
                                max_x,
                                max_y
                            ));
                        }
                    }
                }
            }
        }

        log(&format!(
            "  Glyphs: {}/{} outline ({} with contours, {} empty), {} bitmap",
            parsed_count, total_chars, with_contours, parsed_count - with_contours, bitmap_count
        ));

        log(&format!(
            "[PFR1] '{}': outlineRes={} target_em={}px chars={} outline={} (contours={}) bitmap={} asc={} desc={}",
            font.font_name,
            font.physical_font.outline_resolution,
            target_em_px,
            total_chars,
            parsed_count,
            with_contours,
            bitmap_count,
            font.physical_font.metrics.ascender,
            font.physical_font.metrics.descender,
        ));
    } else {
        log(&format!("  GPS section out of range (offset=0x{:X}, size={}, data_len={})",
            gps_offset, gps_size, data.len()));
    }

    // Case-folding fallback: if a lowercase letter has no contours,
    // copy the uppercase glyph (if available) to the lowercase slot.
    // Shockwave renders uppercase glyphs for missing lowercase chars.
    for lc in b'a'..=b'z' {
        let uc = lc - 32;
        let lc_empty = match font.glyphs.get(&lc) {
            None => true,
            Some(g) => g.contours.is_empty(),
        };
        if lc_empty {
            if let Some(uc_glyph) = font.glyphs.get(&uc).cloned() {
                if !uc_glyph.contours.is_empty() {
                    let mut fallback = uc_glyph;
                    fallback.char_code = lc as u32;
                    font.glyphs.insert(lc, fallback);
                }
            }
        }
    }

    Ok(font)
}

/// Extract font name from PFR data by scanning for readable strings
fn extract_font_name(data: &[u8]) -> Option<String> {
    let mut i = 0;
    while i < data.len().saturating_sub(20) {
        if data[i].is_ascii_alphabetic() {
            let mut name = Vec::new();
            let mut j = i;
            while j < data.len()
                && data[j] != 0
                && (data[j].is_ascii_alphanumeric()
                    || data[j] == b' '
                    || data[j] == b'*'
                    || data[j] == b'_'
                    || data[j] == b'-')
            {
                name.push(data[j]);
                j += 1;
            }
            if name.len() > 3 {
                return Some(String::from_utf8_lossy(&name).to_string());
            }
        }
        i += 1;
    }
    None
}
