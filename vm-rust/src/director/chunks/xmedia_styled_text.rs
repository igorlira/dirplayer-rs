use log::debug;
use crate::player::handlers::datum_handlers::cast_member::font::{StyledSpan, HtmlStyle, TextAlignment};

/// Parsed XMED styled text with all formatting information
pub struct XmedStyledText {
    pub text: String,
    pub styled_spans: Vec<StyledSpan>,
    pub alignment: TextAlignment,
    pub word_wrap: bool,
    pub fixed_line_space: u16,
}

/// Section 3 data - text content
struct Section3Data {
    text: String,
}

/// Section 6 data - character runs mapping text ranges to styles
struct Section6Data {
    char_runs: Vec<CharRun>,
}

/// Character run defining which style applies to a text range
#[derive(Debug, Clone)]
struct CharRun {
    position: u32,
    style_index: u16,
}

/// Section 7 data - style definitions
struct Section7Data {
    styles: Vec<XmedStyle>,
}

/// XMED style definition
#[derive(Debug, Clone)]
struct XmedStyle {
    font_name: String,
    font_size: u16,
    bold: bool,
    italic: bool,
    underline: bool,
    color: Option<u32>,
    word_wrap: bool,
    fore_color: u32,  // dword56 from Section 7 (used in PropertyExtractor line 65)
    back_color: u32,  // dword5E from Section 7 (used in PropertyExtractor lines 66-67)
}

impl Default for XmedStyle {
    fn default() -> Self {
        XmedStyle {
            font_name: "Arial".to_string(),
            font_size: 12,
            bold: false,
            italic: false,
            underline: false,
            color: None,
            word_wrap: true,
            fore_color: 0xFF000000,  // Black with full alpha (default)
            back_color: 0xFFFFFFFF,  // White with full alpha (default)
        }
    }
}

/// Main parser for XMED styled text format
pub fn parse_xmed(data: &[u8]) -> Result<XmedStyledText, String> {
    // Verify magic header "FFFF00000006" (12 ASCII bytes)
    if data.len() < 12 {
        return Err("Data too short for XMED header".to_string());
    }

    if &data[0..4] != b"FFFF" {
        return Err("Invalid XMED magic header".to_string());
    }

    debug!("Parsing XMED format...");
    debug!("  Magic: {:02X?}", &data[0..12]);
    debug!("  Data length: {} bytes", data.len());

    web_sys::console::log_1(&format!("📊 Parsing XMED format: {} bytes", data.len()).into());

    // Show first 40 bytes as hex for debugging
    let preview_len = data.len().min(40);
    let hex_preview: Vec<String> = data[0..preview_len].iter().map(|b| format!("{:02X}", b)).collect();
    web_sys::console::log_1(&format!("  First {} bytes (hex): {}", preview_len, hex_preview.join(" ")).into());

    // Parse sections
    // Format from C# PgReadDoc.Parse():
    // - The magic "FFFF00000006" is PART OF the first 20-byte section header
    // - First header at offset 0: "FFFF" + "00000006" + type + declLen (20 bytes total)
    // - First section has key 0xFFFF, which maps to Section 0 (document header)
    // - All headers are 20 bytes: KKKKCCCCCCCCTTTTDDDD
    //   - 4 chars: section key (hex)
    //   - 8 chars: byte count (hex)
    //   - 4 chars: type (hex)
    //   - 4 chars: declared length (hex)
    // - Then exactly 'count' bytes of Packer-encoded data
    let mut sections: std::collections::HashMap<u16, Vec<u8>> = std::collections::HashMap::new();
    let mut offset = 0; // Start at position 0, magic is part of first header!

    while offset + 20 <= data.len() {
        // Read 20-byte ASCII hex header
        let header_str = String::from_utf8_lossy(&data[offset..offset + 20]);

        // Check for end marker (header contains non-hex characters)
        if !header_str.chars().all(|c| c.is_ascii_hexdigit()) {
            debug!("    Reached end of sections at offset {} (non-ASCII header)", offset);
            web_sys::console::log_1(&format!("  ✅ Reached end of sections at offset {}", offset).into());
            break;
        }

        // Parse 20-byte header: KKKKCCCCCCCCTTTTDDDD
        let key_str = &header_str[0..4];
        let count_str = &header_str[4..12];
        let type_str = &header_str[12..16];
        let decl_len_str = &header_str[16..20];

        // Parse key
        let mut key = match u16::from_str_radix(key_str, 16) {
            Ok(k) => k,
            Err(e) => {
                web_sys::console::log_1(&format!("  ❌ Invalid section key '{}' at offset {}: {}", key_str, offset, e).into());
                break;
            }
        };

        // First section has key 0xFFFF (magic header) - keep it as 0xFFFF
        // Don't convert to 0x0000 to avoid collision with real Section 0

        // Parse byte count (NOT item count!)
        let byte_count = match usize::from_str_radix(count_str, 16) {
            Ok(c) => c,
            Err(e) => {
                web_sys::console::log_1(&format!("  ❌ Invalid byte count '{}' at offset {}: {}", count_str, offset + 4, e).into());
                break;
            }
        };

        let section_type = u16::from_str_radix(type_str, 16).unwrap_or(0);
        let decl_len = usize::from_str_radix(decl_len_str, 16).unwrap_or(0);

        web_sys::console::log_1(&format!("  📦 Section 0x{:04X}: {} bytes, type={}, decl_len={} (header: {})",
                                         key, byte_count, section_type, decl_len, header_str).into());

        offset += 20; // Skip 20-byte ASCII header

        // Read exactly 'byte_count' bytes of section data
        if offset + byte_count > data.len() {
            web_sys::console::log_1(&format!("  ⚠️ Section 0x{:04X} extends beyond data (needs {} bytes at offset {}, have {} remaining)",
                                             key, byte_count, offset, data.len() - offset).into());
            let remaining = data.len() - offset;
            if remaining > 0 {
                sections.insert(key, data[offset..offset + remaining].to_vec());
            }
            break;
        }

        sections.insert(key, data[offset..offset + byte_count].to_vec());
        offset += byte_count;
    }

    web_sys::console::log_1(&format!("  ✅ Found {} sections", sections.len()).into());

    // Parse Section 1 to get document version (int4)
    // C# ProcessSection1 reads from FILE section 0x0000 (C# uses switch(key+1))
    // File section 0x0000 → C# ProcessSection1
    let doc_version: i32 = if let Some(section1) = sections.get(&0x0000) {
        // Debug: show raw bytes
        let preview_len = section1.len().min(20);
        let hex_preview: Vec<String> = section1[0..preview_len].iter().map(|b| format!("{:02X}", b)).collect();
        web_sys::console::log_1(&format!("  📋 Section 1 raw ({} bytes): {}", section1.len(), hex_preview.join(" ")).into());

        let mut packer = Packer::new(section1.clone());
        web_sys::console::log_1(&"  📋 Section 1: Unpacking version with debug...".into());
        let version = packer.unpack_num_debug(true);
        web_sys::console::log_1(&format!("  📋 Section 1: document version = {} (0x{:X}), packer pos after = {}",
                                         version, version, packer.pos).into());
        version
    } else {
        // Default to modern version if Section 1 not found
        web_sys::console::log_1(&"  ⚠️ Section 1 not found, assuming version 262145".into());
        262145
    };

    // Parse text content - can be in Section 2 OR Section 3
    let text_data = if let Some(section2) = sections.get(&0x0002) {
        web_sys::console::log_1(&"  📝 Found text in Section 2".into());
        parse_section_3(section2)?
    } else if let Some(section3) = sections.get(&0x0003) {
        web_sys::console::log_1(&"  📝 Found text in Section 3".into());
        parse_section_3(section3)?
    } else {
        return Err("Missing Section 2 or 3 (text content)".to_string());
    };

    // Parse Section 8 (fonts) - C# switch case 9
    // File section 0x0008 → C# key=8 → switch(key+1)=9 → ProcessSection9
    let font_names = if let Some(section8) = sections.get(&0x0008) {
        web_sys::console::log_1(&"  📝 Found fonts in Section 8".into());
        parse_section_9(section8, doc_version)?
    } else {
        vec!["Arial".to_string()]
    };

    // Parse Section 7 (alignment) - C# switch case 8
    // File section 0x0007 → C# key=7 → switch(key+1)=8 → ProcessSection8
    let alignment = if let Some(section7) = sections.get(&0x0007) {
        web_sys::console::log_1(&"  📐 Found alignment in Section 7".into());
        parse_section_8(section7)?
    } else {
        TextAlignment::Left
    };

    // Parse Section 6 (styles) - C# switch case 7
    // File section 0x0006 → C# key=6 → switch(key+1)=7 → ProcessSection7
    let style_data = if let Some(section6) = sections.get(&0x0006) {
        parse_section_7(section6, &font_names, doc_version)?
    } else {
        Section7Data {
            styles: vec![XmedStyle::default()],
        }
    };

    // Parse Section 5 (character runs) - C# switch case 5 or 6
    // File section 0x0004 or 0x0005 → C# key=4 or 5 → switch(key+1)=5 or 6 → ProcessSection5
    // IMPORTANT: BOTH sections should be processed and COMBINED (C# appends to same _outputItems list)
    let mut all_char_runs = Vec::new();

    if let Some(section4) = sections.get(&0x0004) {
        web_sys::console::log_1(&"  📋 Processing Section 0x0004 (character runs part 1)".into());
        let data4 = parse_section_6(section4)?;
        all_char_runs.extend(data4.char_runs);
    }

    if let Some(section5) = sections.get(&0x0005) {
        web_sys::console::log_1(&"  📋 Processing Section 0x0005 (character runs part 2)".into());
        let data5 = parse_section_6(section5)?;
        all_char_runs.extend(data5.char_runs);
    }

    let char_runs_data = if all_char_runs.is_empty() {
        // Default: all text uses style 0
        Section6Data {
            char_runs: vec![CharRun {
                position: 0,
                style_index: 0,
            }],
        }
    } else {
        Section6Data {
            char_runs: all_char_runs,
        }
    };

    // Convert to StyledSpans
    let styled_spans = create_styled_spans(&text_data.text, &char_runs_data, &style_data);

    // Extract word_wrap and fixed_line_space from the style used by the first character run
    // (matching C# PropertyExtractor which uses _outputItems[0].Value2 as styleIndex)
    let active_style_index = if !char_runs_data.char_runs.is_empty() {
        char_runs_data.char_runs[0].style_index as usize
    } else {
        0
    };

    let active_style = if active_style_index < style_data.styles.len() {
        &style_data.styles[active_style_index]
    } else if !style_data.styles.is_empty() {
        &style_data.styles[0]
    } else {
        &XmedStyle::default()
    };

    let word_wrap = active_style.word_wrap;
    let fixed_line_space = ((active_style.font_size as f32) * 1.2) as u16;

    web_sys::console::log_1(&format!("  🎯 Using style {} from first character run: font='{}', size={}, bold={}, word_wrap={}",
                                     active_style_index, active_style.font_name, active_style.font_size,
                                     active_style.bold, active_style.word_wrap).into());

    debug!("  Parsed: {} chars, {} spans, alignment: {:?}, word_wrap: {}, line_space: {}",
           text_data.text.len(), styled_spans.len(), alignment, word_wrap, fixed_line_space);

    Ok(XmedStyledText {
        text: text_data.text,
        styled_spans,
        alignment,
        word_wrap,
        fixed_line_space,
    })
}

/// Parse Section 3 - Text Content
/// Format: 00 [length], [text] 03
/// Ported from C# Section3TextExtractor.cs ExtractText()
fn parse_section_3(data: &[u8]) -> Result<Section3Data, String> {
    if data.len() < 4 {
        return Err("Section 3 data too short".to_string());
    }

    // Find the comma that separates the length from the text
    let mut comma_pos = None;
    for i in 1..std::cmp::min(10, data.len()) {
        if data[i] == 0x2C {
            // comma
            comma_pos = Some(i);
            break;
        }
    }

    let comma_pos = comma_pos.ok_or("No comma found in Section 3")?;

    // Text starts after the comma
    let text_start = comma_pos + 1;
    let mut text_end = data.len() - 1; // Exclude final 0x03 marker

    if text_end < data.len() && data[text_end] == 0x03 {
        // Good, expected end marker
    } else {
        text_end = data.len(); // No end marker, use full length
    }

    // Extract text, preserving all characters including \r, \n, \t
    let mut text = String::new();
    for i in text_start..text_end {
        text.push(data[i] as char);
    }

    debug!("    Section 3: {} chars", text.len());

    Ok(Section3Data { text })
}

/// Parse Section 6 - Character Runs
/// Each run defines which style applies to text starting at a position
/// Based on C# Sections.cs ProcessSection5 (lines 1228-1236)
fn parse_section_6(data: &[u8]) -> Result<Section6Data, String> {
    let mut packer = Packer::new(data.to_vec());
    let mut char_runs = Vec::new();

    // Read character runs using Packer encoding
    // C# code: while (packer.Remaining > 4) { item.Value1 = packer.UnpackNum(); item.Value2 = packer.UnpackNum(); }
    while packer.remaining() > 0 {
        if packer.remaining() < 2 {
            break; // Not enough data for a full run
        }

        let position = packer.unpack_num() as u32;

        if packer.remaining() < 2 {
            break; // Not enough data for style index
        }

        let style_index = packer.unpack_num() as u16;

        web_sys::console::log_1(&format!("    CharRun: position={}, style_index={}", position, style_index).into());

        char_runs.push(CharRun {
            position,
            style_index,
        });
    }

    debug!("    Section 6: {} character runs", char_runs.len());
    web_sys::console::log_1(&format!("  📋 Section 6: Parsed {} character runs", char_runs.len()).into());

    Ok(Section6Data { char_runs })
}

/// Packer for unpacking variable-length encoded data
/// Ported from C# Packer.cs
struct Packer {
    data: Vec<u8>,
    pos: usize,
    last_value: i32,
    repeat_count: i32,
}

impl Packer {
    fn new(data: Vec<u8>) -> Self {
        Packer {
            data,
            pos: 0,
            last_value: 0,
            repeat_count: 0,
        }
    }

    /// Get remaining bytes in the buffer
    fn remaining(&self) -> usize {
        if self.pos >= self.data.len() {
            0
        } else {
            self.data.len() - self.pos
        }
    }

    /// Check if character is hex digit or minus sign
    fn is_hex_or_minus(c: u8) -> bool {
        (c >= b'0' && c <= b'9') || (c >= b'A' && c <= b'F') || (c >= b'a' && c <= b'f') || c == b'-'
    }

    /// Unpack a single number from the packed data
    /// Ported from C# Packer.cs UnpackNum() method (lines 157-225)
    fn unpack_num(&mut self) -> i32 {
        self.unpack_num_debug(false)
    }

    /// Debug version of unpack_num that can log details
    fn unpack_num_debug(&mut self, debug: bool) -> i32 {
        // Handle repeat mode
        if self.repeat_count > 0 {
            self.repeat_count -= 1;
            if debug {
                web_sys::console::log_1(&format!("    [Packer] Repeat mode, returning last_value={}", self.last_value).into());
            }
            return self.last_value;
        }

        if self.pos >= self.data.len() {
            if debug {
                web_sys::console::log_1(&"    [Packer] pos >= data.len(), returning 0".into());
            }
            return 0;
        }

        let ctrl = self.data[self.pos];
        if debug {
            web_sys::console::log_1(&format!("    [Packer] pos={}, ctrl=0x{:02X} ('{}')",
                self.pos, ctrl, if ctrl >= 0x20 && ctrl < 0x7F { ctrl as char } else { '?' }).into());
        }
        self.pos += 1;

        let mut val: i32 = 0;

        // Check for repeat mode (bit 7 set)
        if (ctrl & 0x80) != 0 {
            val = self.last_value;
            if debug {
                web_sys::console::log_1(&format!("    [Packer] Bit 7 set, repeat mode, val={}", val).into());
            }

            // Check for repeat count (bit 6 set)
            if (ctrl & 0x40) != 0 && self.pos < self.data.len() {
                self.repeat_count = self.data[self.pos] as i32 - 1;
                self.pos += 1;
            }
        } else {
            // Parse hex number from ASCII
            let num_start = self.pos;

            while self.pos < self.data.len() {
                let c = self.data[self.pos];
                if !Self::is_hex_or_minus(c) {
                    break;
                }
                self.pos += 1;
            }

            if self.pos > num_start {
                let hex_bytes = &self.data[num_start..self.pos];
                let hex_str = String::from_utf8_lossy(hex_bytes);
                if debug {
                    web_sys::console::log_1(&format!("    [Packer] Hex string: '{}' ({} chars)", hex_str, hex_str.len()).into());
                }

                let negative = hex_str.starts_with('-');
                let hex_str_clean = if negative {
                    &hex_str[1..]
                } else {
                    &hex_str[..]
                };

                if let Ok(parsed_val) = i32::from_str_radix(hex_str_clean, 16) {
                    val = if negative { -parsed_val } else { parsed_val };
                } else {
                    if debug {
                        web_sys::console::log_1(&format!("    [Packer] Failed to parse hex '{}'", hex_str_clean).into());
                    }
                    val = 0;
                }
            } else {
                if debug {
                    web_sys::console::log_1(&format!("    [Packer] No hex digits found after ctrl byte").into());
                }
            }

            // Handle type code for short values
            let type_code = ctrl & 0x0F;
            if type_code == 1 {
                val = (val as u16) as i32;
                if debug {
                    web_sys::console::log_1(&format!("    [Packer] type_code=1, converted to u16: {}", val).into());
                }
            }
        }

        self.last_value = val;
        val
    }

    /// UnpackRefcon - C# implementation from Sections.cs lines 396-434
    /// If typeCode == 65547 -> use PgUnpackPtrBytes (read size + raw bytes)
    /// Otherwise -> just read one UnpackNum value
    fn unpack_refcon(&mut self, type_code: i32) -> i32 {
        if type_code == 65547 {
            // Special case: use PgUnpackPtrBytes format
            // Format: 0x00 marker, hex-encoded size, then raw bytes
            if self.pos < self.data.len() && self.data[self.pos] == 0x00 {
                self.pos += 1; // Skip marker

                // Read hex-encoded size
                let mut size_str = String::new();
                while self.pos < self.data.len() {
                    let c = self.data[self.pos];
                    if c == b',' || !Self::is_hex_or_minus(c) {
                        break;
                    }
                    size_str.push(c as char);
                    self.pos += 1;
                }

                // Skip comma separator if present
                if self.pos < self.data.len() && self.data[self.pos] == b',' {
                    self.pos += 1;
                }

                // Parse size and skip that many raw bytes
                let size = usize::from_str_radix(&size_str, 10).unwrap_or(0);
                let bytes_consumed = 1 + size_str.len() + 1 + size; // marker + hex + comma + data
                self.pos += size;

                web_sys::console::log_1(&format!("    UnpackRefcon (PtrBytes): size={}, consumed {} bytes", size, bytes_consumed).into());
                return bytes_consumed as i32;
            }
        }

        // Default case: just read one number
        self.unpack_num()
    }
}

/// Parse Section 7 - Style Definitions
/// Ported from C# ProcessSection7 (Sections.cs lines 1243-1372)
/// Uses Packer to extract variable-length encoded style data
fn parse_section_7(data: &[u8], font_names: &[String], doc_version: i32) -> Result<Section7Data, String> {
    let mut packer = Packer::new(data.to_vec());
    let mut styles = Vec::new();

    // Read style count (v50 in C#)
    let style_count = packer.unpack_num();

    if style_count <= 0 || style_count > 100 {
        debug!("    Section 7: Invalid style count {}, using default", style_count);
        return Ok(Section7Data {
            styles: vec![XmedStyle::default()],
        });
    }

    web_sys::console::log_1(&format!("  🎨 Section 7: count={}, doc_version={}", style_count, doc_version).into());

    let mut style_idx = 0;
    // C# code: while (packer.Remaining > 50)
    while packer.remaining() > 50 {
        let mut style = XmedStyle::default();
        let mut parse_failed = false;

        web_sys::console::log_1(&format!("    Style {}: pos={}, remaining={} bytes",
                                         style_idx, packer.pos, packer.remaining()).into());

        // 1. word0 (font_index) - line 1285
        if !parse_failed && packer.remaining() >= 2 {
            let font_index = packer.unpack_num();
            if font_index >= 0 && (font_index as usize) < font_names.len() {
                style.font_name = font_names[font_index as usize].clone();
            }
            web_sys::console::log_1(&format!("    Style {}: word0={} -> font='{}'",
                                             style_idx, font_index, style.font_name).into());
        } else { parse_failed = true; }

        // 2. word42 - line 1286
        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 3. word44 - line 1287
        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 4. word46 (font_size) - line 1288
        if !parse_failed && packer.remaining() >= 2 {
            let font_size = packer.unpack_num();
            if font_size > 0 && font_size <= 200 {
                style.font_size = font_size as u16;
            }
            web_sys::console::log_1(&format!("    Style {}: word46 (fontSize)={}", style_idx, font_size).into());
        } else { parse_failed = true; }

        // 5. word48 (word_wrap: 2=true, 3=false) - line 1289
        if !parse_failed && packer.remaining() >= 2 {
            let word_wrap_value = packer.unpack_num();
            style.word_wrap = word_wrap_value == 2;
        } else { parse_failed = true; }

        // 6. word4A - line 1290
        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 7. word4C - line 1291
        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 8. dword68 - line 1292
        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 9. dword6C - line 1293
        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 10-11. pgUnpackColor (foreColor) - line 1295 (4 values)
        let mut color_values = Vec::new();
        for _ in 0..4 {
            if !parse_failed && packer.remaining() >= 2 {
                color_values.push(packer.unpack_num());
            } else { parse_failed = true; break; }
        }
        if color_values.len() >= 4 {
            let c1 = color_values[0] as u32;
            let c2 = color_values[1] as u32;
            let _c3 = color_values[2] as u32;
            let c4 = color_values[3] as u32;
            style.fore_color = ((c2 >> 8) << 24) | ((c4 >> 8) << 16) | ((c1 >> 8) << 8);
            if (style.fore_color & 0xFF000000) == 0 {
                style.fore_color |= 0xFF000000;
            }
        }

        // 12-13. pgUnpackColor (backColor) - line 1296 (4 values)
        let mut back_color_values = Vec::new();
        for _ in 0..4 {
            if !parse_failed && packer.remaining() >= 2 {
                back_color_values.push(packer.unpack_num());
            } else { parse_failed = true; break; }
        }
        if back_color_values.len() >= 4 {
            let c1 = back_color_values[0] as u32;
            let c2 = back_color_values[1] as u32;
            let _c3 = back_color_values[2] as u32;
            let c4 = back_color_values[3] as u32;
            style.back_color = ((c2 >> 8) << 24) | ((c4 >> 8) << 16) | ((c1 >> 8) << 8);
            if (style.back_color & 0xFF000000) == 0 {
                style.back_color |= 0xFF000000;
            }
        }

        web_sys::console::log_1(&format!("    Style {}: foreColor=0x{:08X}, backColor=0x{:08X}",
                                         style_idx, style.fore_color, style.back_color).into());

        // 14. if (int4 < 65547) dword78 - line 1299-1302
        if !parse_failed && doc_version < 65547 && packer.remaining() >= 2 {
            packer.unpack_num();
        }

        // 15-25. dword80 through dwordA8 (11 values) - lines 1304-1314
        for _ in 0..11 {
            if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }
        }

        // 26. if (int4 < 65547) dword68 again - lines 1316-1319
        if !parse_failed && doc_version < 65547 && packer.remaining() >= 2 {
            packer.unpack_num();
        }

        // 27. if (int4 >= 65551) dwordAC - lines 1321-1324
        if !parse_failed && doc_version >= 65551 && packer.remaining() >= 2 {
            packer.unpack_num();
        }

        // 28. UnpackRefcon - line 1327
        if !parse_failed && packer.remaining() >= 2 {
            packer.unpack_refcon(doc_version);
        } else { parse_failed = true; }

        // 29. dword120 - line 1329
        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 30. gapB4 = UnpackNumber(8, 2) - line 1331 (8 values)
        for _ in 0..8 {
            if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }
        }

        // 31. gap2 = UnpackNumber(32, 1) - line 1336 (32 values) - CRITICAL for bold/italic/underline!
        // C# calculates: v3 = (int4 >= 257) ? 0 : -1; v3 = v3 & 0xF0; count = v3 + 32
        // For version 262145 >= 257, v3 = 0, so count = 32
        let gap2_count = if doc_version >= 257 { 32 } else { 32 - 16 }; // v3 & 0xF0 = -16 when v3=-1

        if !parse_failed {
            web_sys::console::log_1(&format!("    Style {}: Reading gap2 ({} values), remaining {} bytes",
                                             style_idx, gap2_count, packer.remaining()).into());
            let mut gap2 = Vec::new();
            for i in 0..gap2_count {
                if packer.remaining() >= 2 {
                    gap2.push(packer.unpack_num());
                } else {
                    web_sys::console::log_1(&format!("    Style {}: Ran out at gap2[{}]", style_idx, i).into());
                    break;
                }
            }

            if gap2.len() >= 3 {
                style.bold = gap2[0] == 1;
                style.italic = gap2[1] == 1;
                style.underline = gap2[2] == 1;
                web_sys::console::log_1(&format!("    Style {}: gap2[0-2]=[{},{},{}] -> bold={}, italic={}, underline={}",
                                                 style_idx, gap2[0], gap2[1], gap2[2],
                                                 style.bold, style.italic, style.underline).into());
            }
        }

        // 32. if (int4 >= 65536) dword74 - lines 1343-1346
        if !parse_failed && doc_version >= 65536 && packer.remaining() >= 2 {
            packer.unpack_num();
        }

        // 33-36. if (int4 >= 65552) dwordB0, word4E, word50, word54 - lines 1348-1354
        if !parse_failed && doc_version >= 65552 {
            for _ in 0..4 {
                if packer.remaining() >= 2 { packer.unpack_num(); }
            }
        }

        // 37. if (int4 >= 65555) dword70 - lines 1356-1359
        if !parse_failed && doc_version >= 65555 && packer.remaining() >= 2 {
            packer.unpack_num();
        }

        web_sys::console::log_1(&format!("    Style {}: FINAL -> font='{}', size={}, bold={}, italic={}, underline={}",
                                         style_idx, style.font_name, style.font_size,
                                         style.bold, style.italic, style.underline).into());

        styles.push(style);
        style_idx += 1;
    }

    web_sys::console::log_1(&format!("  ✅ Section 7: Parsed {} style(s) (initial count was {})", styles.len(), style_count).into());

    if styles.is_empty() {
        styles.push(XmedStyle::default());
    }

    debug!("    Section 7: Parsed {} style(s)", styles.len());

    Ok(Section7Data { styles })
}

/// Parse Section 8 - Paragraph Runs (Alignment)
/// Based on C# Section8RawComparison.cs findings:
/// - Left: Section size <= 36 bytes (no second paragraph run)
/// - Center: byte[36] = 0x31 ('1')
/// - Right: byte[36] = 0x32 ('2')
/// - Justify: byte[36] = 0x33 ('3')
fn parse_section_8(data: &[u8]) -> Result<TextAlignment, String> {
    web_sys::console::log_1(&format!("  📐 Parsing alignment: {} bytes", data.len()).into());

    // Left-aligned: 36 bytes or less (single paragraph run, alignment implicit=0)
    if data.len() <= 36 {
        web_sys::console::log_1(&format!("  📐 Alignment: Left (section size {} <= 36)", data.len()).into());
        return Ok(TextAlignment::Left);
    }

    // Center/Right/Justify: more than 36 bytes with alignment at byte[36]
    let alignment_byte = data[36];
    let alignment = match alignment_byte {
        0x31 => {
            web_sys::console::log_1(&"  📐 Alignment: Center (byte[36]=0x31 '1')".into());
            TextAlignment::Center
        }
        0x32 => {
            web_sys::console::log_1(&"  📐 Alignment: Right (byte[36]=0x32 '2')".into());
            TextAlignment::Right
        }
        0x33 => {
            web_sys::console::log_1(&"  📐 Alignment: Justify (byte[36]=0x33 '3')".into());
            TextAlignment::Justify
        }
        _ => {
            web_sys::console::log_1(&format!("  📐 Alignment: Left (byte[36]=0x{:02X} unknown)", alignment_byte).into());
            TextAlignment::Left
        }
    };

    Ok(alignment)
}

/// Font information from Section 9
#[derive(Debug, Clone)]
struct FontInfo {
    name: String,
    kerning: bool,
    anti_alias: bool,
}

/// Parse Section 9 - Font Definitions
/// Based on C# Sections.cs ProcessSection9 (lines 1610-1729)
/// Font names stored using PgUnpackPtrBytes format: 00 [hex_size] [font_name_bytes]
fn parse_section_9(data: &[u8], doc_version: i32) -> Result<Vec<String>, String> {
    web_sys::console::log_1(&format!("  📝 Parsing Section 9 (Font Definitions): {} bytes", data.len()).into());

    let mut font_names = Vec::new();
    let mut font_infos = Vec::new();
    let mut offset = 0;

    // Check if we have font names (starts with 0x00 marker)
    if data.is_empty() || data[0] != 0x00 {
        web_sys::console::log_1(&"    Section 9: No font names (no 0x00 marker), using default".into());
        return Ok(vec!["Arial".to_string()]);
    }

    // C# code: for (int i = 0; i < a4; i++) where a4 = 2
    // Each font ENTRY has:
    //   - First font name (64 bytes: 00 + hex_size + comma + data)
    //   - Second font name (64 bytes, usually empty)
    //   - Properties (Packer-encoded, ~38 bytes)
    // Total per entry: ~174 bytes
    for entry_idx in 0..2 {
        if offset >= data.len() {
            break;
        }

        web_sys::console::log_1(&format!("  🔤 Font Entry {}: Starting at offset {}", entry_idx, offset).into());

        // Read FIRST font name for this entry
        match read_font_name(data, &mut offset, entry_idx, 0) {
            Ok(Some(name)) => {
                web_sys::console::log_1(&format!("    Entry {}, Name 1: '{}' at offset {}", entry_idx, name, offset).into());
                font_names.push(name);
            }
            Ok(None) => {
                web_sys::console::log_1(&format!("    Entry {}, Name 1: (empty)", entry_idx).into());
            }
            Err(e) => {
                web_sys::console::log_1(&format!("    Entry {}: Error reading first name: {}", entry_idx, e).into());
                break;
            }
        }

        // Read SECOND font name for this entry (usually empty)
        // C# code: if (Sections._section1.int4 >= 65550) { ... } - lines 1648-1656
        if doc_version >= 65550 {
            match read_font_name(data, &mut offset, entry_idx, 1) {
                Ok(Some(name)) => {
                    if !name.is_empty() {
                        web_sys::console::log_1(&format!("    Entry {}, Name 2: '{}' (unusual - second name not empty!)", entry_idx, name).into());
                        font_names.push(name);
                    }
                }
                Ok(None) => {
                    // Expected - second name is usually empty
                }
                Err(e) => {
                    web_sys::console::log_1(&format!("    Entry {}: Error reading second name: {}", entry_idx, e).into());
                    break;
                }
            }
        }

        // Read properties section by parsing with Packer to advance offset correctly
        // C# reads: word80-word8a, dword90, UnpackRefcon, UnpackNumber(8,2), dword8C, word86
        let (kerning, anti_alias) = match read_font_properties(data, &mut offset, entry_idx, doc_version) {
            Ok((font_style, anti_alias_val, kerning_val)) => {
                // Properties read successfully, offset now points to next entry
                // Extract boolean values (C# PropertyExtractor uses: fontInfo.word88 > 0, fontInfo.word8A > 0)
                let kerning = kerning_val > 0;
                let anti_alias = anti_alias_val > 0;

                web_sys::console::log_1(&format!("    Entry {}: kerning={}, antiAlias={}",
                                                 entry_idx, kerning, anti_alias).into());
                (kerning, anti_alias)
            }
            Err(e) => {
                web_sys::console::log_1(&format!("    Entry {}: Error reading properties: {}, stopping", entry_idx, e).into());
                break;
            }
        };

        // Store font info with properties if we have a font name for this entry
        if entry_idx < font_names.len() {
            font_infos.push(FontInfo {
                name: font_names[entry_idx].clone(),
                kerning,
                anti_alias,
            });
        }
    }

    if font_names.is_empty() {
        web_sys::console::log_1(&"    Section 9: No fonts parsed, using default 'Arial'".into());
        font_names.push("Arial".to_string());
    }

    web_sys::console::log_1(&format!("  ✅ Section 9: Parsed {} font name(s) with properties", font_names.len()).into());

    // For now, just return font names for compatibility with Section 7 parsing
    // TODO: Expose kerning and anti_alias properties in XmedStyledText or StyledSpan
    Ok(font_names)
}

/// Read font properties section and advance offset
/// Ported from C# ProcessSection9 property reading (lines 1670-1718)
/// Returns (font_style, anti_alias, kerning)
fn read_font_properties(data: &[u8], offset: &mut usize, entry_idx: usize, doc_version: i32) -> Result<(u16, u16, u16), String> {
    if *offset >= data.len() {
        return Err(format!("Offset {} beyond data length {}", offset, data.len()));
    }

    let start_offset = *offset;

    // Create a Packer starting at current offset
    let remaining_data = data[*offset..].to_vec();
    let mut packer = Packer::new(remaining_data);

    web_sys::console::log_1(&format!("    Entry {}: Reading properties, doc_version={}", entry_idx, doc_version).into());

    // C# ProcessSection9 reads (lines 1671-1710):

    // 1. word80 (fontStyle) - line 1671
    let word80 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    // 2. word82 (fontSize) - line 1675
    let word82 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    // 3. word84 (charSpacing) - line 1679
    let word84 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    // 4. word88 (kerning) - line 1683
    let word88 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    // 5. if (int4 >= 65552) word8A (antiAlias) - lines 1686-1691
    let word8a = if doc_version >= 65552 && packer.remaining() >= 2 {
        packer.unpack_num() as u16
    } else {
        0
    };

    // 6. dword90 - line 1694
    let _dword90 = if packer.remaining() >= 2 { packer.unpack_num() as u32 } else { 0 };

    // 7. UnpackRefcon - line 1698
    if packer.remaining() >= 2 {
        packer.unpack_refcon(doc_version);
    }

    // 8. word94 = UnpackNumber(8, 2) - line 1699 (8 values)
    for i in 0..8 {
        if packer.remaining() >= 2 {
            packer.unpack_num();
        } else {
            web_sys::console::log_1(&format!("    Entry {}: Ran out at word94[{}]", entry_idx, i).into());
            break;
        }
    }

    // 9-10. if (int4 >= 256) dword8C, word86 - lines 1707-1711
    if doc_version >= 256 {
        if packer.remaining() >= 2 { packer.unpack_num(); } // dword8C
        if packer.remaining() >= 2 { packer.unpack_num(); } // word86
    }

    // Advance offset by how much the packer consumed
    let consumed = packer.pos;
    *offset += consumed;

    web_sys::console::log_1(&format!("    Entry {}: fontStyle=0x{:04X}, fontSize={}, kerning=0x{:04X}, antiAlias=0x{:04X}, consumed {} bytes",
                                     entry_idx, word80, word82, word88, word8a, consumed).into());

    Ok((word80, word8a, word88))
}

/// Helper function to read a single font name using PgUnpackPtrBytes format
/// Returns Ok(Some(name)) if found, Ok(None) if empty, Err if parse error
fn read_font_name(data: &[u8], offset: &mut usize, entry_idx: usize, name_idx: usize) -> Result<Option<String>, String> {
    if *offset >= data.len() {
        return Err(format!("Offset {} beyond data length {}", offset, data.len()));
    }

    // Check for 0x00 marker
    if data[*offset] != 0x00 {
        return Err(format!("No 0x00 marker at offset {}", offset));
    }

    *offset += 1; // Skip marker

    // Read ASCII hex size (e.g., "40" = 64 decimal, "40," would be 3 chars)
    let hex_start = *offset;
    while *offset < data.len() && is_hex_digit(data[*offset]) {
        *offset += 1;
    }

    if *offset == hex_start {
        return Err("No hex size found".to_string());
    }

    let hex_str = String::from_utf8_lossy(&data[hex_start..*offset]);
    let size = match usize::from_str_radix(&hex_str, 16) {
        Ok(s) => s,
        Err(e) => {
            return Err(format!("Invalid hex size '{}': {}", hex_str, e));
        }
    };

    // Skip comma if present (C# trace shows: "Read size from stream: 64 (hex length: 3)")
    // The hex length is 3 because it includes the comma after "40" → "40,"
    if *offset < data.len() && data[*offset] == b',' {
        *offset += 1;
    }

    // Read font name bytes
    if *offset + size > data.len() {
        return Err(format!("Not enough data: need {} bytes at offset {}, have {} remaining",
                          size, *offset, data.len() - *offset));
    }

    let font_bytes = &data[*offset..*offset + size];

    // C# extracts string, then trims null bytes
    // Font bytes format: [length_byte] [name_bytes] [null_padding]
    // E.g., 05 41 72 69 61 6C 00 00... = length 5, "Arial", padding
    let font_name = if size > 0 && font_bytes[0] > 0 {
        let name_len = font_bytes[0] as usize;
        if name_len + 1 <= size {
            String::from_utf8_lossy(&font_bytes[1..1 + name_len]).to_string()
        } else {
            // Fallback: just trim nulls from entire buffer
            String::from_utf8_lossy(font_bytes).trim_end_matches('\0').to_string()
        }
    } else {
        // No length byte or zero length - empty font name
        String::new()
    };

    *offset += size; // Advance by full chunk size (64 bytes)

    if font_name.is_empty() {
        Ok(None)
    } else {
        Ok(Some(font_name))
    }
}

/// Check if byte is ASCII hex digit
fn is_hex_digit(b: u8) -> bool {
    (b >= b'0' && b <= b'9') || (b >= b'A' && b <= b'F') || (b >= b'a' && b <= b'f')
}

/// Convert character runs and styles into StyledSpan entries
fn create_styled_spans(
    text: &str,
    char_runs: &Section6Data,
    styles: &Section7Data,
) -> Vec<StyledSpan> {
    let mut spans = Vec::new();

    if char_runs.char_runs.is_empty() {
        // Default: entire text with default style
        return vec![StyledSpan {
            text: text.to_string(),
            style: xmed_style_to_html_style(&XmedStyle::default()),
        }];
    }

    // Sort runs by position and deduplicate (keep first run at each position)
    let mut sorted_runs = char_runs.char_runs.clone();
    sorted_runs.sort_by_key(|r| r.position);

    // Deduplicate: when multiple runs have the same position, keep only the first one
    let mut deduped_runs: Vec<CharRun> = Vec::new();
    let mut last_position: Option<u32> = None;
    for run in sorted_runs {
        if last_position != Some(run.position) {
            last_position = Some(run.position);
            deduped_runs.push(run);
        }
    }

    // Create spans for each run
    for (i, run) in deduped_runs.iter().enumerate() {
        let start = run.position as usize;
        let end = if i + 1 < deduped_runs.len() {
            deduped_runs[i + 1].position as usize
        } else {
            text.len()
        };

        if start >= text.len() {
            break;
        }

        let span_text = text[start..std::cmp::min(end, text.len())].to_string();
        if span_text.is_empty() {
            continue;
        }

        let style_index = run.style_index as usize;
        let style = if style_index < styles.styles.len() {
            &styles.styles[style_index]
        } else {
            &styles.styles[0]
        };

        spans.push(StyledSpan {
            text: span_text,
            style: xmed_style_to_html_style(style),
        });
    }

    if spans.is_empty() {
        // Fallback: entire text with default style
        spans.push(StyledSpan {
            text: text.to_string(),
            style: xmed_style_to_html_style(&XmedStyle::default()),
        });
    }

    spans
}

/// Convert XMED style to HtmlStyle format
fn xmed_style_to_html_style(xmed_style: &XmedStyle) -> HtmlStyle {
    HtmlStyle {
        font_face: Some(xmed_style.font_name.clone()),
        font_size: Some(xmed_style.font_size as i32),
        color: xmed_style.color,
        bg_color: None,
        bold: xmed_style.bold,
        italic: xmed_style.italic,
        underline: xmed_style.underline,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_section_3_simple() {
        // "Test Text" = 9 chars
        let data = vec![
            0x00, 0x39, 0x2C, // Start marker + "9,"
            b'T', b'e', b's', b't', b' ', b'T', b'e', b'x', b't', // "Test Text"
            0x03, // End marker
        ];

        let result = parse_section_3(&data).unwrap();
        assert_eq!(result.text, "Test Text");
    }

    #[test]
    fn test_parse_section_8_left() {
        // 36 bytes or less = left alignment
        let data = vec![0u8; 36];
        let alignment = parse_section_8(&data).unwrap();
        assert!(matches!(alignment, TextAlignment::Left));
    }

    #[test]
    fn test_parse_section_8_center() {
        // 72 bytes with 0x31 at position 36 = center
        let mut data = vec![0u8; 72];
        data[36] = 0x31;
        let alignment = parse_section_8(&data).unwrap();
        assert!(matches!(alignment, TextAlignment::Center));
    }

    #[test]
    fn test_parse_section_8_right() {
        // 72 bytes with 0x32 at position 36 = right
        let mut data = vec![0u8; 72];
        data[36] = 0x32;
        let alignment = parse_section_8(&data).unwrap();
        assert!(matches!(alignment, TextAlignment::Right));
    }

    #[test]
    fn test_parse_section_8_justify() {
        // 72 bytes with 0x33 at position 36 = justify
        let mut data = vec![0u8; 72];
        data[36] = 0x33;
        let alignment = parse_section_8(&data).unwrap();
        assert!(matches!(alignment, TextAlignment::Justify));
    }

    #[test]
    fn test_packer_unpack_simple_hex() {
        // Simple hex number: control byte 0x01, then "C" (hex for 12)
        let data = vec![0x01, b'C'];
        let mut packer = Packer::new(data);
        let result = packer.unpack_num();
        assert_eq!(result, 0x0C); // 12 in decimal
    }

    #[test]
    fn test_packer_unpack_multi_digit_hex() {
        // Multi-digit hex: control byte 0x01, then "10" (hex for 16)
        let data = vec![0x01, b'1', b'0'];
        let mut packer = Packer::new(data);
        let result = packer.unpack_num();
        assert_eq!(result, 0x10); // 16 in decimal
    }

    #[test]
    fn test_packer_repeat_mode() {
        // First value: control 0x01, hex "5"
        // Second value: control 0x80 (repeat last value)
        let data = vec![0x01, b'5', 0x80];
        let mut packer = Packer::new(data);

        let first = packer.unpack_num();
        assert_eq!(first, 5);

        let second = packer.unpack_num();
        assert_eq!(second, 5); // Should repeat
    }

    #[test]
    fn test_packer_repeat_with_count() {
        // First value: control 0x01, hex "A" (10)
        // Repeat 3 times: control 0xC0 (bit 7 and 6 set), count byte 0x03
        let data = vec![0x01, b'A', 0xC0, 0x03];
        let mut packer = Packer::new(data);

        let first = packer.unpack_num();
        assert_eq!(first, 10);

        // Should repeat 2 more times (count - 1)
        let second = packer.unpack_num();
        assert_eq!(second, 10);

        let third = packer.unpack_num();
        assert_eq!(third, 10);
    }

    #[test]
    fn test_packer_gap2_bold() {
        // Simulate Section 7 data with gap2[0]=1 (bold)
        // Format: style_count=1, then 32 gap2 values starting with 1,0,0...
        let mut data = vec![0x01, b'1']; // style_count = 1

        // gap2[0] = 1 (bold)
        data.extend_from_slice(&[0x01, b'1']);

        // gap2[1] = 0 (not italic)
        data.extend_from_slice(&[0x01, b'0']);

        // gap2[2] = 0 (not underline)
        data.extend_from_slice(&[0x01, b'0']);

        // Remaining 29 gap2 values as 0 (using repeat mode)
        data.extend_from_slice(&[0xC0, 29]); // Repeat last value (0) 28 more times

        let mut packer = Packer::new(data);
        let style_count = packer.unpack_num();
        assert_eq!(style_count, 1);

        let mut gap2 = Vec::new();
        for _ in 0..32 {
            gap2.push(packer.unpack_num());
        }

        assert_eq!(gap2[0], 1); // bold
        assert_eq!(gap2[1], 0); // not italic
        assert_eq!(gap2[2], 0); // not underline
    }

    #[test]
    fn test_packer_gap2_italic() {
        // Simulate Section 7 data with gap2[1]=1 (italic)
        let mut data = vec![0x01, b'1']; // style_count = 1

        // gap2[0] = 0 (not bold)
        data.extend_from_slice(&[0x01, b'0']);

        // gap2[1] = 1 (italic)
        data.extend_from_slice(&[0x01, b'1']);

        // gap2[2] = 0 (not underline)
        data.extend_from_slice(&[0x01, b'0']);

        // Remaining gap2 values as 0
        data.extend_from_slice(&[0xC0, 29]);

        let mut packer = Packer::new(data);
        let style_count = packer.unpack_num();
        assert_eq!(style_count, 1);

        let mut gap2 = Vec::new();
        for _ in 0..32 {
            gap2.push(packer.unpack_num());
        }

        assert_eq!(gap2[0], 0); // not bold
        assert_eq!(gap2[1], 1); // italic
        assert_eq!(gap2[2], 0); // not underline
    }

    #[test]
    fn test_packer_gap2_underline() {
        // Simulate Section 7 data with gap2[2]=1 (underline)
        let mut data = vec![0x01, b'1']; // style_count = 1

        // gap2[0] = 0 (not bold)
        data.extend_from_slice(&[0x01, b'0']);

        // gap2[1] = 0 (not italic)
        data.extend_from_slice(&[0x01, b'0']);

        // gap2[2] = 1 (underline)
        data.extend_from_slice(&[0x01, b'1']);

        // Remaining gap2 values as 0
        data.extend_from_slice(&[0xC0, 29]);

        let mut packer = Packer::new(data);
        let style_count = packer.unpack_num();
        assert_eq!(style_count, 1);

        let mut gap2 = Vec::new();
        for _ in 0..32 {
            gap2.push(packer.unpack_num());
        }

        assert_eq!(gap2[0], 0); // not bold
        assert_eq!(gap2[1], 0); // not italic
        assert_eq!(gap2[2], 1); // underline
    }

    #[test]
    fn test_parse_real_xmed_hx_dat04() {
        // Real XMED data from hx_dat04.txt: "Tool Needed\rPlanks" with Arial/Verdana fonts
        // This is actual binary data from a Director file
        let hex_string = "46 46 46 46 30 30 30 30 30 30 30 36 30 30 30 34 30 30 30 31 01 37 37 41 41 03 30 30 30 30 30 30 30 30 30 30 35 43 30 30 30 30 30 30 30 30 02 34 30 30 30 31 02 31 30 31 02 2D 37 46 46 43 36 46 45 30 02 30 C2 03 02 34 38 30 30 34 38 02 2D 31 02 30 02 31 38 01 30 C1 03 02 2D 31 82 02 37 02 44 02 37 02 44 02 30 C2 06 02 31 38 02 36 32 01 39 39 30 30 01 46 46 30 30 81 01 30 82 82 02 31 02 30 C2 04 02 31 32 01 30 03 30 30 30 31 30 30 30 30 30 30 33 35 30 30 30 30 30 30 30 31 02 30 02 31 32 01 38 34 02 30 01 32 02 30 01 32 02 30 82 82 02 31 38 02 36 32 01 43 01 30 01 32 01 38 45 30 30 01 30 02 43 02 31 45 02 31 38 02 34 34 02 30 03 30 30 30 32 30 30 30 30 30 30 31 37 30 30 30 30 30 30 30 30 00 31 32 2C 54 6F 6F 6C 20 4E 65 65 64 65 64 0D 50 6C 61 6E 6B 73 03 30 30 30 34 30 30 30 30 30 30 30 41 30 30 30 30 30 30 30 32 02 30 01 33 02 31 34 01 33 03 30 30 30 35 30 30 30 30 30 30 30 41 30 30 30 30 30 30 30 32 02 30 01 31 02 31 34 01 30 03 30 30 30 36 30 30 30 30 30 30 43 41 30 30 30 30 30 30 30 34 01 33 01 30 81 81 01 43 01 33 01 30 81 82 82 C1 04 01 46 46 46 46 81 81 01 30 82 02 43 30 30 30 30 02 30 C2 0A 02 31 02 30 C2 07 C1 20 82 82 C1 03 82 01 31 01 30 81 01 43 01 32 01 30 81 82 82 C1 04 01 46 46 46 46 81 81 01 30 82 02 43 30 30 30 30 02 30 C2 0A 02 32 02 30 C2 07 C1 20 82 82 C1 03 82 01 31 01 30 81 01 41 01 32 01 30 81 82 82 C1 04 01 46 46 46 46 81 81 01 30 82 02 41 30 30 30 30 02 30 C2 0A 02 33 02 30 C2 07 C1 20 82 82 C1 03 82 01 31 01 30 81 01 41 01 32 01 30 81 82 82 C1 04 01 46 46 46 46 81 81 01 30 82 02 41 30 30 30 30 02 30 C2 0A 02 36 02 30 C2 07 01 31 01 30 C1 1E 82 82 C1 03 82 03 30 30 30 37 30 30 30 30 30 30 34 38 30 30 30 30 30 30 30 32 01 30 81 81 C2 0F 81 82 02 31 02 30 C2 06 02 36 41 30 33 45 32 41 45 01 30 02 31 38 01 30 82 C1 03 C2 12 01 31 01 30 81 C2 0F 81 82 02 31 02 30 C2 06 02 36 41 30 33 45 32 41 45 01 30 02 31 38 01 30 82 C1 03 C2 12 03 30 30 30 38 30 30 30 30 30 31 37 30 30 30 30 30 30 30 30 32 00 34 30 2C 05 41 72 69 61 6C 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 34 30 2C 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 31 30 34 01 30 01 37 44 31 01 30 02 34 45 34 02 34 30 30 02 30 82 82 02 31 02 30 02 34 30 30 02 34 30 30 30 38 02 30 82 02 31 30 31 01 30 00 34 30 2C 07 56 65 72 64 61 6E 61 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 34 30 2C 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 00 01 31 30 34 01 30 01 31 43 32 33 01 30 02 34 45 34 02 34 30 30 02 30 82 82 02 31 02 30 02 34 30 30 02 34 30 30 30 38 02 30 82 02 31 30 31 01 30 03 30 30 30 39 30 30 30 30 30 30 31 33 30 30 30 30 30 30 30 32 02 30 82 02 31 38 02 36 32 02 30 82 02 31 38 02 36 32 03 30 30 30 41 30 30 30 30 30 30 31 33 30 30 30 30 30 30 30 32 02 30 82 02 31 38 02 36 32 02 30 82 02 31 38 02 36 32 03 30 30 30 42 30 30 30 30 30 30 30 35 30 30 30 30 30 30 30 32 02 30 C2 07 03 30 30 30 43 30 30 30 30 30 30 31 34 30 30 30 30 30 30 30 31 02 31 32 02 31 82 02 38 02 31 32 01 30 02 34 33 82 01 31 03 30 30 30 46 30 30 30 30 30 30 32 31 30 30 30 30 30 30 30 30 02 34 30 01 30 C2 0B 01 31 02 30 C2 09 81 81 82 81 81 C2 04 C1 05 C2 03 C1 04 82 82 C1 03 C2 20 03 30 30 31 33 30 30 30 30 30 30 37 45 30 30 30 30 30 30 30 30 01 44 01 41 01 39 01 31 46 01 38 01 43 01 45 01 31 43 01 31 44 01 31 45 01 31 46 01 37 46 01 31 42 01 30";

        // Parse hex string into bytes
        let bytes: Vec<u8> = hex_string
            .split_whitespace()
            .filter_map(|s| u8::from_str_radix(s, 16).ok())
            .collect();

        // Parse XMED data
        let result = parse_xmed(&bytes);
        assert!(result.is_ok(), "Failed to parse XMED data: {:?}", result.err());

        let xmed = result.unwrap();

        // Verify text content with preserved carriage return
        assert_eq!(xmed.text, "Tool Needed\rPlanks", "Text should be 'Tool Needed\\rPlanks' with carriage return preserved");
        assert_eq!(xmed.text.len(), 18, "Text length should be 18 chars");

        // Verify text contains carriage return
        assert!(xmed.text.contains('\r'), "Text should contain carriage return");

        // Verify alignment (should be left for this file)
        assert!(matches!(xmed.alignment, TextAlignment::Left), "Alignment should be Left");

        // Verify styled spans were created
        assert!(!xmed.styled_spans.is_empty(), "Should have styled spans");

        // Verify first span properties
        let first_span = &xmed.styled_spans[0];
        assert!(first_span.style.font_face.is_some(), "Should have font face");

        // Font name should be Arial or Verdana (from Section 9)
        let font_name = first_span.style.font_face.as_ref().unwrap();
        assert!(
            font_name == "Arial" || font_name == "Verdana",
            "Font should be Arial or Verdana, got: {}",
            font_name
        );

        // Verify font size is reasonable
        assert!(first_span.style.font_size.is_some(), "Should have font size");
        let font_size = first_span.style.font_size.unwrap();
        assert!(font_size > 0 && font_size < 200, "Font size should be reasonable, got: {}", font_size);

        println!("✅ Real XMED data test passed:");
        println!("   Text: {:?}", xmed.text);
        println!("   Length: {} chars", xmed.text.len());
        println!("   Alignment: {:?}", xmed.alignment);
        println!("   Spans: {}", xmed.styled_spans.len());
        println!("   Font: {:?}", first_span.style.font_face);
        println!("   Size: {:?}", first_span.style.font_size);
        println!("   Bold: {}", first_span.style.bold);
        println!("   Italic: {}", first_span.style.italic);
        println!("   Underline: {}", first_span.style.underline);
    }
}
