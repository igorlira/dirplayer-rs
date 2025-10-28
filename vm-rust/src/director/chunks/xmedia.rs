use binary_reader::{BinaryReader, Endian};
use web_sys::console;

// Import the PFR vector renderer
use super::pfr_renderer::render_pfr_font;

pub struct XMediaChunk {
    pub raw_data: Vec<u8>,
}

pub struct PfrFont {
    pub font_name: String,
    pub char_width: u16,
    pub char_height: u16,
    pub glyph_data: Vec<u8>,
    pub grid_columns: u8,
    pub grid_rows: u8,
}

impl XMediaChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<XMediaChunk, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let mut raw_data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            raw_data.push(byte);
        }

        reader.endian = original_endian;

        console::log_1(&format!("XMED raw_data ({} bytes)", raw_data.len()).into());

        Ok(XMediaChunk { raw_data })
    }

    pub fn is_pfr_font(&self) -> bool {
        if self.raw_data.len() < 100 {
            return false;
        }

        // Check for "PFR1" magic (0x50 0x46 0x52 0x31)
        if self.raw_data.len() >= 4 && &self.raw_data[0..4] == b"PFR1" {
            console::log_1(&"✓ Found PFR1 magic header".into());
            return true;
        }

        // Reject styled text XMedia chunks (start with "FFFF")
        if self.raw_data.len() >= 4 && &self.raw_data[0..4] == b"FFFF" {
            console::log_1(&"✗ This is styled text data (FFFF header), not a font".into());
            return false;
        }

        false
    }

    pub fn extract_font_name(&self) -> Option<String> {
        // Look for null-terminated strings that might be font names
        let mut i = 0;
        while i < self.raw_data.len() - 20 {
            // Check if this looks like a font name (starts with printable ASCII)
            if self.raw_data[i].is_ascii_alphabetic() {
                let mut name = Vec::new();
                let mut j = i;

                // Collect until null or non-printable
                while j < self.raw_data.len()
                    && self.raw_data[j] != 0
                    && (self.raw_data[j].is_ascii_alphanumeric()
                        || self.raw_data[j] == b' '
                        || self.raw_data[j] == b'*'
                        || self.raw_data[j] == b'_')
                {
                    name.push(self.raw_data[j]);
                    j += 1;
                }

                if name.len() > 3 {
                    let name_str = String::from_utf8_lossy(&name).to_string();
                    if name_str.contains("FFF") || name_str.contains("Reaction") {
                        return Some(name_str);
                    }
                }
            }
            i += 1;
        }
        None
    }

    fn looks_like_bitmap_data(data: &[u8], bytes_per_glyph: usize) -> bool {
        // Check if this looks like real bitmap data
        // Real 1-bit bitmap fonts have bytes where each bit is a pixel
        // So values are typically in range 0x00-0xFF but distributed normally
        // Vector commands use lots of high bytes (0x80+) as opcodes

        if data.len() < bytes_per_glyph * 80 {
            console::log_1(&"      Fail: Not enough data for 80 glyphs".into());
            return false;
        }

        // Real bitmap data should have LOW bytes dominating
        // Count bytes in different ranges
        let sample_size = data.len().min(512);
        let very_low = data[0..sample_size].iter().filter(|&&b| b < 0x40).count();
        let low = data[0..sample_size].iter().filter(|&&b| b < 0x80).count();
        let high = data[0..sample_size].iter().filter(|&&b| b >= 0x80).count();

        let very_low_ratio = very_low as f32 / sample_size as f32;
        let low_ratio = low as f32 / sample_size as f32;

        // Real bitmap data should have >40% very low bytes (0x00-0x3F)
        // and >60% low bytes (0x00-0x7F)
        if very_low_ratio < 0.4 {
            console::log_1(
                &format!(
                    "      Fail: Only {:.1}% very low bytes (need >40%)",
                    very_low_ratio * 100.0
                )
                .into(),
            );
            return false;
        }

        if low_ratio < 0.6 {
            console::log_1(
                &format!(
                    "      Fail: Only {:.1}% low bytes (need >60%)",
                    low_ratio * 100.0
                )
                .into(),
            );
            return false;
        }

        // Check it's not all the same value
        let first_byte = data[0];
        let all_same = data[0..sample_size].iter().all(|&b| b == first_byte);
        if all_same {
            console::log_1(&format!("      Fail: All bytes are 0x{:02X}", first_byte).into());
            return false;
        }

        console::log_1(
            &format!(
                "      Pass: {:.1}% very_low, {:.1}% low, {:.1}% high bytes",
                very_low_ratio * 100.0,
                low_ratio * 100.0,
                high as f32 / sample_size as f32 * 100.0
            )
            .into(),
        );
        true
    }

    pub fn parse_pfr_font(&self) -> Option<PfrFont> {
        if !self.is_pfr_font() {
            return None;
        }

        console::log_1(&"🔍 Parsing PFR1 font format...".into());

        // Extract font name
        let font_name = self
            .extract_font_name()
            .unwrap_or_else(|| format!("Unknown_PFR_Font"));

        console::log_1(&format!("  Font name: '{}'", font_name).into());

        // Character dimensions are stored as single bytes:
        // Width at offset 0x56, Height at offset 0x58
        let char_width = if self.raw_data.len() > 0x56 {
            let w = self.raw_data[0x56] as u16;
            if w > 0 && w <= 32 {
                w
            } else {
                8
            }
        } else {
            8
        };

        let char_height = if self.raw_data.len() > 0x58 {
            let h = self.raw_data[0x58] as u16;
            if h > 0 && h <= 32 {
                h
            } else {
                8
            }
        } else {
            8
        };

        console::log_1(&format!("  Char dimensions: {}×{} pixels", char_width, char_height).into());

        // Grid is 16×8 for 128 characters
        let grid_columns = 16u8;
        let grid_rows = 8u8;

        // Calculate expected data size
        let bytes_per_row = ((char_width as usize + 7) / 8);
        let bytes_per_glyph = bytes_per_row * char_height as usize;
        let total_glyphs = grid_columns as usize * grid_rows as usize;
        let expected_bitmap_bytes = bytes_per_glyph * total_glyphs;

        console::log_1(
            &format!(
                "  Expected: {} bytes/glyph, {} total bytes needed",
                bytes_per_glyph, expected_bitmap_bytes
            )
            .into(),
        );

        // Try multiple offsets to find the bitmap data
        let candidate_offsets = vec![
            0x200,                                                     // Common PFR glyph data offset
            0x400,                                                     // 1KB boundary
            0x800,                                                     // 2KB boundary
            0xC00,                                                     // 3KB boundary
            self.raw_data.len().saturating_sub(expected_bitmap_bytes), // End of file
        ];

        console::log_1(
            &format!(
                "  Searching for bitmap data at {} candidate offsets...",
                candidate_offsets.len()
            )
            .into(),
        );

        let mut glyph_data = None;
        let mut found_offset = 0;

        // First try the known offsets
        for &offset in &candidate_offsets {
            if offset + expected_bitmap_bytes <= self.raw_data.len() {
                let candidate = &self.raw_data[offset..offset + expected_bitmap_bytes];

                // Log what we're checking
                console::log_1(
                    &format!(
                        "  Checking offset 0x{:04X}: first 16 bytes: {:02X?}",
                        offset,
                        &candidate[0..16.min(candidate.len())]
                    )
                    .into(),
                );

                // Check 'H' character (ASCII 72) specifically
                let h_offset = 72 * bytes_per_glyph;
                if h_offset + bytes_per_glyph <= candidate.len() {
                    let h_glyph = &candidate[h_offset..h_offset + bytes_per_glyph];
                    console::log_1(&format!("    'H' (glyph #72): {:02X?}", h_glyph).into());
                }

                // Also check space for reference
                let space_offset = 32 * bytes_per_glyph;
                if space_offset + bytes_per_glyph <= candidate.len() {
                    let space_glyph = &candidate[space_offset..space_offset + bytes_per_glyph];
                    console::log_1(&format!("    Space (glyph #32): {:02X?}", space_glyph).into());
                }

                if Self::looks_like_bitmap_data(candidate, bytes_per_glyph) {
                    console::log_1(
                        &format!("  ✓ Found bitmap data at offset 0x{:04X}!", offset).into(),
                    );
                    glyph_data = Some(candidate.to_vec());
                    found_offset = offset;
                    break;
                } else {
                    console::log_1(&"    ✗ Validation failed".into());
                }
            } else {
                console::log_1(
                    &format!(
                        "  Skipping offset 0x{:04X}: not enough data (need {}, have {})",
                        offset,
                        expected_bitmap_bytes,
                        self.raw_data.len().saturating_sub(offset)
                    )
                    .into(),
                );
            }
        }

        // If none of the standard offsets worked, try a brute force scan
        // looking for sections where bytes are mostly in the 0x00-0x7F range
        if glyph_data.is_none() {
            console::log_1(&"  No bitmap at standard offsets, trying brute-force scan...".into());

            let mut best_offset = 0;
            let mut best_score = 0.0;

            // Scan every 8-byte boundary
            let mut scan_offset = 0x100;
            while scan_offset + expected_bitmap_bytes <= self.raw_data.len() {
                let candidate = &self.raw_data[scan_offset..scan_offset + expected_bitmap_bytes];

                // Score based on how "bitmap-like" it looks
                let sample_size = candidate.len().min(256);
                let low_bytes = candidate[0..sample_size]
                    .iter()
                    .filter(|&&b| b < 0x80)
                    .count();
                let score = low_bytes as f32 / sample_size as f32;

                if score > best_score {
                    best_score = score;
                    best_offset = scan_offset;
                }

                scan_offset += 8;
            }

            if best_score > 0.5 {
                console::log_1(
                    &format!(
                        "  ✓ Brute-force found candidate at offset 0x{:04X} (score: {:.1}%)",
                        best_offset,
                        best_score * 100.0
                    )
                    .into(),
                );

                let candidate = &self.raw_data[best_offset..best_offset + expected_bitmap_bytes];
                glyph_data = Some(candidate.to_vec());
                found_offset = best_offset;
            }
        }

        if glyph_data.is_none() {
            console::log_1(&"  ✗ No pre-rendered bitmap data found.".into());
            console::log_1(&"  🎨 Attempting to render from PFR vector data...".into());

            // Try to render using the vector renderer
            let glyph_data_offset = 0x200; // Vector data starts here
            if glyph_data_offset < self.raw_data.len() {
                let vector_data = &self.raw_data[glyph_data_offset..];

                console::log_1(
                    &format!(
                        "  📐 Rendering {} glyphs at {}×{} from {} bytes of vector data",
                        total_glyphs,
                        char_width,
                        char_height,
                        vector_data.len()
                    )
                    .into(),
                );

                let rendered_bitmap = render_pfr_font(
                    vector_data,
                    char_width as usize,
                    char_height as usize,
                    total_glyphs,
                );

                if rendered_bitmap.len() >= expected_bitmap_bytes {
                    console::log_1(&"  ✅ Successfully rendered PFR vector font!".into());
                    glyph_data = Some(rendered_bitmap[..expected_bitmap_bytes].to_vec());
                    found_offset = glyph_data_offset;
                } else {
                    console::log_1(
                        &format!(
                            "  ⚠️  Rendered {} bytes, expected {}",
                            rendered_bitmap.len(),
                            expected_bitmap_bytes
                        )
                        .into(),
                    );
                }
            }
        }

        if glyph_data.is_none() {
            console::log_1(&"  ✗ Could not render PFR font.".into());
            console::log_1(&"  ".into());
            console::log_1(&"  ═══════════════════════════════════════════════════════".into());
            console::log_1(&"  ⚠️  PFR RENDERING FAILED".into());
            console::log_1(&"  ═══════════════════════════════════════════════════════".into());
            console::log_1(&"  System font will be used as fallback.".into());
            console::log_1(&"  ═══════════════════════════════════════════════════════".into());
            return None;
        }

        let glyph_data = glyph_data.unwrap();

        console::log_1(
            &format!(
                "  ✅ Extracted {} bytes of bitmap data from offset 0x{:04X}",
                glyph_data.len(),
                found_offset
            )
            .into(),
        );

        // Log first glyph of space character for verification
        let space_offset = 32 * bytes_per_glyph;
        if space_offset + bytes_per_glyph <= glyph_data.len() {
            console::log_1(
                &format!(
                    "  📋 Space char (glyph #32): {:02X?}",
                    &glyph_data[space_offset..space_offset + bytes_per_glyph.min(8)]
                )
                .into(),
            );
        }

        Some(PfrFont {
            font_name,
            char_width,
            char_height,
            glyph_data,
            grid_columns,
            grid_rows,
        })
    }

    pub fn decode_pfr_rle_bitmap(
        glyph_data: &[u8],
        expected_width: u16,
        expected_height: u16,
    ) -> Vec<u8> {
        console::log_1(
            &format!(
                "🔧 Decoding PFR RLE bitmap: {} bytes → {}×{} pixels",
                glyph_data.len(),
                expected_width,
                expected_height
            )
            .into(),
        );

        let total_pixels = expected_width as usize * expected_height as usize;
        let mut bitmap = vec![0u8; total_pixels / 8]; // 1 bit per pixel
        let mut bit_pos = 0;
        let mut pos = 0;

        // PFR RLE encoding uses command bytes:
        // 0x00-0x7F: literal byte follows
        // 0x80-0xFF: repeat next byte (count = value - 0x80)
        // OR: pairs of (count, value)

        // Try simple RLE: byte pairs (count, value)
        while pos < glyph_data.len() - 1 && bit_pos < total_pixels {
            let count = glyph_data[pos] as usize;
            let value = glyph_data[pos + 1];

            // Write 'count' bits of 'value'
            for _ in 0..count.min(8) {
                if bit_pos >= total_pixels {
                    break;
                }

                let byte_idx = bit_pos / 8;
                let bit_idx = 7 - (bit_pos % 8);

                if byte_idx < bitmap.len() {
                    if value > 0 {
                        bitmap[byte_idx] |= 1 << bit_idx;
                    }
                }

                bit_pos += 1;
            }

            pos += 2;
        }

        console::log_1(
            &format!(
                "   Decoded {} pixels ({} bits, {} bytes)",
                bit_pos,
                bit_pos,
                bitmap.len()
            )
            .into(),
        );

        bitmap
    }

    pub fn parse_pfr_font_with_rle(&self) -> Option<PfrFont> {
        if !self.is_pfr_font() {
            return None;
        }

        console::log_1(&"🔍 Parsing PFR1 font format with RLE...".into());

        // ... (same font name extraction as before) ...

        let char_width = 8u16;
        let char_height = 12u16;
        let grid_columns = 16u8;
        let grid_rows = 8u8;

        let bitmap_width = char_width * grid_columns as u16;
        let bitmap_height = char_height * grid_rows as u16;

        // Extract RLE data starting from offset 0x200 or detected offset
        let glyph_data_offset = 0x200;
        let rle_data = if glyph_data_offset < self.raw_data.len() {
            &self.raw_data[glyph_data_offset..]
        } else {
            &self.raw_data[..]
        };

        // Decode RLE to raw bitmap
        let decoded_bitmap = Self::decode_pfr_rle_bitmap(rle_data, bitmap_width, bitmap_height);

        console::log_1(
            &format!(
                "  ✅ PFR font: {}×{} chars, grid {}×{}, {} bytes decoded bitmap",
                char_width,
                char_height,
                grid_columns,
                grid_rows,
                decoded_bitmap.len()
            )
            .into(),
        );

        let font_name = self
            .extract_font_name()
            .unwrap_or_else(|| format!("Unknown_PFR_Font"));

        console::log_1(&format!("  Font name: '{}'", font_name).into());

        Some(PfrFont {
            font_name,
            char_width,
            char_height,
            glyph_data: decoded_bitmap,
            grid_columns,
            grid_rows,
        })
    }
}
