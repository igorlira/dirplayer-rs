use log::debug;
use crate::player::handlers::datum_handlers::cast_member::font::{StyledSpan, HtmlStyle, TextAlignment};

/// Mac Roman to Unicode mapping for bytes 0x80-0xFF.
/// Director files from Mac use Mac Roman encoding.
/// Source: https://www.unicode.org/Public/MAPPINGS/VENDORS/APPLE/ROMAN.TXT
const MAC_ROMAN_TABLE: [char; 128] = [
    '\u{00C4}', '\u{00C5}', '\u{00C7}', '\u{00C9}', '\u{00D1}', '\u{00D6}', '\u{00DC}', '\u{00E1}', // 80-87
    '\u{00E0}', '\u{00E2}', '\u{00E4}', '\u{00E3}', '\u{00E5}', '\u{00E7}', '\u{00E9}', '\u{00E8}', // 88-8F
    '\u{00EA}', '\u{00EB}', '\u{00ED}', '\u{00EC}', '\u{00EE}', '\u{00EF}', '\u{00F1}', '\u{00F3}', // 90-97
    '\u{00F2}', '\u{00F4}', '\u{00F6}', '\u{00F5}', '\u{00FA}', '\u{00F9}', '\u{00FB}', '\u{00FC}', // 98-9F
    '\u{2020}', '\u{00B0}', '\u{00A2}', '\u{00A3}', '\u{00A7}', '\u{2022}', '\u{00B6}', '\u{00DF}', // A0-A7
    '\u{00AE}', '\u{00A9}', '\u{2122}', '\u{00B4}', '\u{00A8}', '\u{2260}', '\u{00C6}', '\u{00D8}', // A8-AF
    '\u{221E}', '\u{00B1}', '\u{2264}', '\u{2265}', '\u{00A5}', '\u{00B5}', '\u{2202}', '\u{2211}', // B0-B7
    '\u{220F}', '\u{03C0}', '\u{222B}', '\u{00AA}', '\u{00BA}', '\u{03A9}', '\u{00E6}', '\u{00F8}', // B8-BF
    '\u{00BF}', '\u{00A1}', '\u{00AC}', '\u{221A}', '\u{0192}', '\u{2248}', '\u{2206}', '\u{00AB}', // C0-C7
    '\u{00BB}', '\u{2026}', '\u{00A0}', '\u{00C0}', '\u{00C3}', '\u{00D5}', '\u{0152}', '\u{0153}', // C8-CF
    '\u{2013}', '\u{2014}', '\u{201C}', '\u{201D}', '\u{2018}', '\u{2019}', '\u{00F7}', '\u{25CA}', // D0-D7
    '\u{00FF}', '\u{0178}', '\u{2044}', '\u{20AC}', '\u{2039}', '\u{203A}', '\u{FB01}', '\u{FB02}', // D8-DF
    '\u{2021}', '\u{00B7}', '\u{201A}', '\u{201E}', '\u{2030}', '\u{00C2}', '\u{00CA}', '\u{00C1}', // E0-E7
    '\u{00CB}', '\u{00C8}', '\u{00CD}', '\u{00CE}', '\u{00CF}', '\u{00CC}', '\u{00D3}', '\u{00D4}', // E8-EF
    '\u{F8FF}', '\u{00D2}', '\u{00DA}', '\u{00DB}', '\u{00D9}', '\u{0131}', '\u{02C6}', '\u{02DC}', // F0-F7
    '\u{00AF}', '\u{02D8}', '\u{02D9}', '\u{02DA}', '\u{00B8}', '\u{02DD}', '\u{02DB}', '\u{02C7}', // F8-FF
];

fn mac_roman_to_char(byte: u8) -> char {
    if byte < 0x80 {
        byte as char
    } else {
        MAC_ROMAN_TABLE[(byte - 0x80) as usize]
    }
}

impl XmedStyledText {
    /// Resolve the active `par_info` for a given text byte/char position.
    /// Walks `par_runs` (sorted by `position`) and returns the par_info
    /// referenced by the run with the largest position ≤ `pos`. Falls
    /// back to `par_infos[0]` (the document default) when no run applies.
    pub fn par_info_at(&self, pos: u32) -> Option<&ParInfo> {
        let mut active: Option<&ParRun> = None;
        for run in &self.par_runs {
            if run.position <= pos {
                active = Some(run);
            } else {
                break;
            }
        }
        let idx = active.map(|r| r.par_info_index as usize).unwrap_or(0);
        self.par_infos.get(idx)
    }

    /// Effective `line_spacing` for a given text position, with the
    /// "0 means inherit / use document default" rule applied. Returns
    /// 0 only when no par_info has a non-zero spacing.
    pub fn line_spacing_at(&self, pos: u32) -> i32 {
        if let Some(pi) = self.par_info_at(pos) {
            if pi.line_spacing != 0 {
                return pi.line_spacing;
            }
        }
        // When the line's par_info has line_spacing=0 ("inherit"),
        // Director uses the MAX non-zero line_spacing across the member
        // as the document default. Verified against Junkbot v1 level.num
        // (par_infos = [0, 16, 21, 0], 16 lines): Director reports
        // `line[1].fixedLineSpace = 21` even though par_run[0] points to
        // par_info[0] (= 0); the renderer's page_height = 15 × 21 + 16 =
        // 331 confirms the layout — most lines stride at the max value
        // (21), with the explicit non-default 16 reserved for the last
        // line. A "first non-zero" fallback would have returned 16, off
        // by 5 px per line.
        self.par_infos
            .iter()
            .map(|pi| pi.line_spacing)
            .filter(|&s| s != 0)
            .max()
            .unwrap_or(0)
    }
}

/// One paragraph's formatting record (Paige `par_info`). Section 0x0007
/// stores N of these; Section 0x0005's `par_run` entries point into this
/// table by index. The renderer / `member.line[N].fixedLineSpace` getter
/// looks up which par_info applies to a given text position via par_runs,
/// then reads `line_spacing` (Paige `dword270`).
#[derive(Debug, Clone)]
pub struct ParInfo {
    pub line_spacing: i32,
    pub line_height: i32,
    pub left_indent: i32,
    pub right_indent: i32,
    pub first_indent: i32,
    pub top_spacing: i32,
    pub bottom_spacing: i32,
    /// Paige par_info `justification` (word0): 0=left, 1=center, 2=right, 3=justify.
    pub justification: i32,
}

impl Default for ParInfo {
    fn default() -> Self {
        ParInfo {
            line_spacing: 0,
            line_height: 0,
            left_indent: 0,
            right_indent: 0,
            first_indent: 0,
            top_spacing: 0,
            bottom_spacing: 0,
            justification: 0,
        }
    }
}

/// Paragraph run: maps a text offset to a par_info index. Stored in
/// XMED Section 0x0005 (Paige `par_run_key`). Same wire format as
/// style_run_key but the index is into `par_infos`, not `styles`.
#[derive(Debug, Clone)]
pub struct ParRun {
    pub position: u32,
    pub par_info_index: u16,
}

/// Parsed XMED styled text with all formatting information
pub struct XmedStyledText {
    pub text: String,
    pub styled_spans: Vec<StyledSpan>,
    pub alignment: TextAlignment,
    pub word_wrap: bool,
    pub fixed_line_space: u16,
    // Text member properties (from Section 1)
    pub width: i32,
    pub height: i32,
    pub page_height: i32,
    // Field properties (from Section 8)
    pub line_height: i32,
    pub line_count: i32,
    // Paragraph formatting (from Section 8 paragraph runs)
    pub left_indent: i32,
    pub right_indent: i32,
    pub first_indent: i32,
    pub line_spacing: i32,
    pub top_spacing: i32,
    pub bottom_spacing: i32,
    // Per-paragraph formatting: Section 0x0007 stores N par_info entries;
    // Section 0x0005 (par_run_key) maps text offsets to indices in this
    // table. Use `line_spacing_at(pos)` to resolve per-line line_spacing.
    pub par_infos: Vec<ParInfo>,
    pub par_runs: Vec<ParRun>,
    // Background color (from Section 0x0000 document header, indices 30-32)
    pub bg_color: Option<(u8, u8, u8)>,
    /// Member-level "default" font size, sourced from XMED Section 0x0005's
    /// first character run → its `style_index` → Section 7's `font_size`.
    /// In Paige enum order 0x0005 is `par_run_key`; XMED reverses the role —
    /// Director's `the fontSize of member` reads through this section's
    /// first run (verified against FurniFactory clock display: 0x0004 said
    /// 15, 0x0005 said 12, Director reports 12).
    /// `None` when 0x0005 is absent — the consumer should fall back to the
    /// existing per-span font size.
    pub default_font_size: Option<u16>,
}

/// Section 1 data - document header with page/field properties
struct Section1Data {
    doc_version: i32,
    width: i32,
    height: i32,
    page_height: i32,
    bg_color: Option<(u8, u8, u8)>,
    /// HTML `<font size=N>` attribute (1..7) saved into the doc header by
    /// Director when a member.html setter assigns text without authoring
    /// a Section 7 style entry. Only meaningful when Section 7 declares
    /// 0 styles — otherwise the per-style font_size in Section 7 is the
    /// authoritative source. Verified empirically against members 36
    /// (`<font size=6>` → val=6 → 24 pt) vs 82 (no size attr → val=0 →
    /// engine default 12 pt).
    html_font_size_attr: i32,
}

impl Default for Section1Data {
    fn default() -> Self {
        Section1Data {
            doc_version: 262145,  // Default modern version
            width: 0,
            height: 0,
            page_height: 0,
            bg_color: None,
            html_font_size_attr: 0,
        }
    }
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

/// Sort runs by position and collapse duplicates at the same position.
/// Keep the first run for each position (section 0x0004 precedence over 0x0005).
fn normalize_char_runs(runs: &[CharRun]) -> Vec<CharRun> {
    let mut sorted = runs.to_vec();
    sorted.sort_by_key(|r| r.position);

    let mut normalized: Vec<CharRun> = Vec::new();
    for run in sorted {
        if let Some(last) = normalized.last() {
            if last.position == run.position {
                continue;
            }
        }
        normalized.push(run);
    }
    normalized
}

/// Section 7 data - style definitions
struct Section7Data {
    styles: Vec<XmedStyle>,
    /// Style count declared in the section header. The packed data often
    /// contains MORE styles than this (we keep parsing past the count
    /// because char-run arrays can reference later styles), but Director
    /// itself appears to treat any styles past the declared count as
    /// invalid for purposes of `member.fontSize` defaulting. When this is
    /// 0, the member has no authored style and the size lookup must fail
    /// so the consumer falls back to the engine default.
    declared_style_count: usize,
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
    kerning: i32,     // dword98 from Section 7 (stored as fixed-point * 65536, divide to get value)
    char_spacing: i32, // dword9C from Section 7 (stored as fixed-point * 65536, divide to get pixels)
    /// True if this style's `word0` (font index) was within the parsed
    /// Section 9 font-name table when the style was loaded. Used by the
    /// member-level fontSize logic to decide whether the 0x0004 char run's
    /// style is "valid" enough to source a fontSize from, or whether to
    /// fall back to 0x0005's first run instead. (Director appears to skip
    /// style runs whose font_index is out of range when computing
    /// `member.fontSize`.)
    font_index_valid: bool,
    /// Raw `word0` font index from Section 7. Section 9 in some XMED chunks
    /// declares many more fonts than are realistically authored (member 74
    /// declares 15 entries even though only Arial+Verdana are real
    /// references), so `font_index_valid` (a `< font_names.len()` check)
    /// can spuriously accept synthesised styles like style[4].word0=14.
    /// The HTML-map detector uses this raw value with a small threshold to
    /// distinguish authored styles (word0 0..~7) from synthesised noise.
    font_index_raw: i32,
    /// Raw `word48` from Section 7. When this style was authored from an
    /// HTML `<font size=N>` tag, word48 is non-zero (encodes a per-style
    /// size class). When the wrapping `<font>` tag had no `size=`
    /// attribute, word48 is 0 — Director then falls back to the document
    /// default (style[0].word46) for that char's `member.fontSize` rather
    /// than the per-style word46. Verified empirically against member 16
    /// (CS credits screen, all `<font>` tags lack `size=`, every char
    /// run resolves to a style with word48=0, member.fontSize → 12).
    word48_raw: i32,
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
            kerning: 0,              // Default: no kerning adjustment
            char_spacing: 0,         // Default: no extra spacing
            font_index_valid: false, // No referenced font for the default
            font_index_raw: -1,
            word48_raw: 0,
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

    debug!("Parsing XMED format: {} bytes", data.len());

    // Show first 40 bytes as hex for debugging
    let preview_len = data.len().min(40);
    let hex_preview: Vec<String> = data[0..preview_len].iter().map(|b| format!("{:02X}", b)).collect();
    debug!("  First {} bytes (hex): {}", preview_len, hex_preview.join(" "));

    // Parse sections
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
    // Section 0x0002 (text content) can span multiple chunks that need concatenation.
    // Parse each chunk individually and accumulate the text.
    let mut section2_texts: Vec<String> = Vec::new();
    let mut offset = 0; // Start at position 0, magic is part of first header!

    while offset + 20 <= data.len() {
        // Read 20-byte ASCII hex header
        let header_str = String::from_utf8_lossy(&data[offset..offset + 20]);

        // Check for end marker (header contains non-hex characters)
        if !header_str.chars().all(|c| c.is_ascii_hexdigit()) {
            debug!("    Reached end of sections at offset {} (non-ASCII header)", offset);
            debug!("Reached end of sections at offset {}", offset);
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
                debug!("Invalid section key '{}' at offset {}: {}", key_str, offset, e);
                break;
            }
        };

        // First section has key 0xFFFF (magic header) - keep it as 0xFFFF
        // Don't convert to 0x0000 to avoid collision with real Section 0

        // Parse byte count (NOT item count!)
        let byte_count = match usize::from_str_radix(count_str, 16) {
            Ok(c) => c,
            Err(e) => {
                debug!("Invalid byte count '{}' at offset {}: {}", count_str, offset + 4, e);
                break;
            }
        };

        let section_type = u16::from_str_radix(type_str, 16).unwrap_or(0);
        let decl_len = usize::from_str_radix(decl_len_str, 16).unwrap_or(0);

        debug!("Section 0x{:04X}: {} bytes, type={}, decl_len={} (header: {})",
                                         key, byte_count, section_type, decl_len, header_str);

        offset += 20; // Skip 20-byte ASCII header

        // Read exactly 'byte_count' bytes of section data
        if offset + byte_count > data.len() {
            debug!("Section 0x{:04X} extends beyond data (needs {} bytes at offset {}, have {} remaining)",
                                             key, byte_count, offset, data.len() - offset);
            let remaining = data.len() - offset;
            if remaining > 0 {
                sections.insert(key, data[offset..offset + remaining].to_vec());
            }
            break;
        }

        let chunk = data[offset..offset + byte_count].to_vec();
        // Section 0x0002 can have multiple chunks; parse each and accumulate text
        if key == 0x0002 {
            if let Ok(parsed) = parse_section_3(&chunk) {
                section2_texts.push(parsed.text);
            }
        }
        sections.insert(key, chunk);
        offset += byte_count;
    }

    debug!("Found {} sections", sections.len());

    // Parse Section 1 to get document header (version, width, height, pageHeight)
    let section1_data = if let Some(section1) = sections.get(&0x0000) {
        parse_section_1(section1)?
    } else {
        // Default to modern version if Section 1 not found
        debug!("Section 1 not found, using defaults");
        Section1Data::default()
    };
    let doc_version = section1_data.doc_version;

    // Parse text content - can be in Section 2 (possibly multi-chunk) OR Section 3
    // Each Section 2 chunk represents one line of text. Join them with \r (Director's
    // Mac-heritage line break character) to restore the original line structure.
    // Compute chunk boundaries for adjusting character run positions later.
    // When we join Section 2 chunks with \r, character run positions (which reference
    // the original concatenated-without-\r text) need to be shifted forward.
    // Each boundary is the cumulative length of chunks 0..i (without \r separators).
    let section2_boundaries: Vec<u32> = if section2_texts.len() > 1 {
        let mut boundaries = Vec::with_capacity(section2_texts.len() - 1);
        let mut cumulative = 0u32;
        for chunk_text in &section2_texts[..section2_texts.len() - 1] {
            cumulative += chunk_text.len() as u32;
            boundaries.push(cumulative);
        }
        boundaries
    } else {
        Vec::new()
    };

    let text_data = if !section2_texts.is_empty() {
        debug!("Found text in Section 2 ({} chunk(s))", section2_texts.len());
        Section3Data { text: section2_texts.join("\r") }
    } else if let Some(section3) = sections.get(&0x0003) {
        debug!("Found text in Section 3");
        parse_section_3(section3)?
    } else {
        return Err("Missing Section 2 or 3 (text content)".to_string());
    };

    // Parse Section 8 (fonts)
    // File section 0x0008 ProcessSection9
    let font_names = if let Some(section8) = sections.get(&0x0008) {
        debug!("Found fonts in Section 8");
        parse_section_9(section8, doc_version)?
    } else {
        vec!["Arial".to_string()]
    };

    // Parse Section 7 (alignment and paragraph formatting)
    // File section 0x0007 ProcessSection8
    let paragraph_info = if let Some(section7) = sections.get(&0x0007) {
        debug!("Found paragraph runs in Section 7");
        parse_section_8(section7, doc_version)?
    } else {
        ParagraphInfo::default()
    };

    // Per-paragraph par_info table. Section 0x0007 holds N par_info
    // records (Paige `par_info` struct, 748 bytes when expanded but
    // packed via Packer at ~30-50 bytes each). Section 0x0005 stores
    // (text_offset, par_info_idx) pairs that point into this table.
    // Used by `XmedStyledText::line_spacing_at(pos)` and the
    // `member.line[N].fixedLineSpace` getter.
    let par_infos: Vec<ParInfo> = if let Some(section7) = sections.get(&0x0007) {
        parse_section_8_par_infos(section7, doc_version)
    } else {
        Vec::new()
    };

    // Member-level alignment is computed below, after section5_char_runs
    // (the par_run table) is loaded — we look up par_info[par_run[0].idx]
    // to get the first paragraph's justification.
    let mut alignment = paragraph_info.alignment;

    // Parse Section 6 (styles)
    // File section 0x0006 ProcessSection7
    let style_data = if let Some(section6) = sections.get(&0x0006) {
        parse_section_7(section6, &font_names, doc_version)?
    } else {
        Section7Data {
            styles: vec![XmedStyle::default()],
            declared_style_count: 0,
        }
    };

    // Parse Section 0x0004 (style_runs: position → style_index) and
    // Section 0x0005 (par_runs: position → par_info_index). These two
    // sections share the (offset, index) wire format but the indices
    // point into DIFFERENT tables — Section 7 styles vs. Section 8
    // par_infos. They must be kept separate.
    //
    // We previously concatenated them into `all_char_runs` and used the
    // union for styled_spans, which corrupted span colors at positions
    // where a par_run landed (Junkbot help member 139: par_run at pos
    // 378 with par_info_index=1 was misread as style_index=1, splitting
    // the eyeBOT title across two spans with the wrong style).
    let mut section4_char_runs: Vec<CharRun> = Vec::new();
    let mut section5_char_runs: Vec<CharRun> = Vec::new();

    if let Some(section4) = sections.get(&0x0004) {
        debug!("Processing Section 0x0004 (style_runs)");
        let data4 = parse_section_6(section4)?;
        section4_char_runs = data4.char_runs.clone();
    }

    if let Some(section5) = sections.get(&0x0005) {
        debug!("Processing Section 0x0005 (par_runs)");
        let data5 = parse_section_6(section5)?;
        section5_char_runs = data5.char_runs.clone();
    }

    // Styled spans are driven solely by style_runs (Section 0x0004).
    let mut all_char_runs: Vec<CharRun> = section4_char_runs.clone();

    // Adjust character run positions to account for \r characters inserted between
    // Section 2 chunks. Each boundary represents a point where a \r was inserted,
    // so positions at or after boundary[i] need to be shifted by (i+1).
    if !section2_boundaries.is_empty() {
        for run in &mut all_char_runs {
            // Count how many chunk boundaries are <= this position
            let shift = section2_boundaries.iter()
                .filter(|&&b| b <= run.position)
                .count() as u32;
            run.position += shift;
        }
        for run in &mut section5_char_runs {
            let shift = section2_boundaries.iter()
                .filter(|&&b| b <= run.position)
                .count() as u32;
            run.position += shift;
        }
    }

    // Member-level alignment: look up the par_info referenced by the
    // FIRST par_run (Section 0x0005 entry at position 0) — that's the
    // alignment of the first paragraph, which Director reports as
    // `member.alignment`. par_info[0] is a template default; par_info[1+]
    // hold per-paragraph overrides. par_run[0] points to whichever
    // par_info applies to text offset 0.
    //
    //   Junkbot brick info member 139: par_runs=[(0,0), (N,1), …]
    //     par_run[0] → par_info[0].justification=0 → Left ✓
    //   Centered title member: par_runs=[(0,1)]
    //     par_run[0] → par_info[1].justification=1 → Center ✓
    //
    // par_info[0]'s value alone was wrong because it's just a template;
    // the heuristic byte-36 reader was wrong because it indexes into the
    // SECOND par_info (which holds a per-paragraph override).
    if let Some(par_run0) = section5_char_runs.first() {
        let idx = par_run0.style_index as usize;
        if let Some(pi) = par_infos.get(idx) {
            alignment = match pi.justification {
                1 => TextAlignment::Center,
                2 => TextAlignment::Right,
                3 => TextAlignment::Justify,
                _ => TextAlignment::Left,
            };
        }
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

    // Extract word_wrap and fixed_line_space from the style used by the first effective
    // character run after position-sort + duplicate resolution.
    let normalized_runs = normalize_char_runs(&char_runs_data.char_runs);
    let active_style_index = if !normalized_runs.is_empty() {
        normalized_runs[0].style_index as usize
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

    let word_wrap = if let Some(run) = section5_char_runs.first() {
        let idx = run.style_index as usize;
        if idx < style_data.styles.len() {
            style_data.styles[idx].word_wrap
        } else {
            active_style.word_wrap
        }
    } else {
        active_style.word_wrap
    };
    // Director's `member.fixedLineSpace` reads the line_spacing of the
    // par_info that par_run[0] points to — i.e. the paragraph applied at
    // text position 0. This is the dominant authored stride for the
    // member as a whole.
    //
    // Verified against Junkbot v1 level.num / level.name / level.moves
    // (par_infos = [0, 16, 21, 0], par_run[0] → par_info[2] = 21):
    // Director reports `member.fixedLineSpace = 21`. And Junkbot v1
    // brick-info member 139 (par_run[0] → par_info[0] = 0): Director
    // reports `member.fixedLineSpace = 0` so per-line par_info values
    // drive the rendering (including the special 6 for the empty line
    // before "eyeBOT").
    //
    // The legacy heuristic at `parse_section_8` (reading `paragraph_info
    // .line_spacing` from a hard-coded index in the packed value stream)
    // was unreliable — it landed on 16 for member 14 and on 21 for
    // members 15/16, even though all three have identical par_info
    // structures. par_run[0] is the authoritative source.
    let fixed_line_space: u16 = if !section5_char_runs.is_empty() && !par_infos.is_empty() {
        // par_run[0]'s par_info is the authoritative member-level
        // line_spacing. Trust its value INCLUDING zero — a zero here
        // means "no member-level stride; per-paragraph par_infos drive
        // rendering". Junkbot brick info members (120 'HELP_text_1',
        // 121 'HELP_text_2', 139 EYEBOT) all have par_run[0] →
        // par_info[0] = 0 and rely on per-line par_info gaps for
        // empty-paragraph spacing; without trusting the 0 the legacy
        // heuristic below picks up unrelated values (HELP_text_1 lands
        // on 11) and forces every line to that stride.
        section5_char_runs
            .first()
            .and_then(|cr| par_infos.get(cr.style_index as usize))
            .map(|pi| pi.line_spacing as u16)
            .unwrap_or(0)
    } else if paragraph_info.line_spacing > 0 {
        // Older / simpler XMED members without par_runs+par_infos: use
        // the legacy `parse_section_8` heuristic (single ParagraphInfo).
        paragraph_info.line_spacing as u16
    } else {
        0
    };

    debug!("Using style {} from first character run: font='{}', size={}, bold={}, word_wrap={}",
                                     active_style_index, active_style.font_name, active_style.font_size,
                                     active_style.bold, active_style.word_wrap);

    debug!("  Parsed: {} chars, {} spans, alignment: {:?}, word_wrap: {}, line_space: {}",
           text_data.text.len(), styled_spans.len(), alignment, word_wrap, fixed_line_space);

    debug!(
        "XmedStyledText result: text='{}' alignment={:?} word_wrap={} fixed_line_space={} \
         width={} height={} page_height={} line_height={} line_count={} \
         left_indent={} right_indent={} first_indent={} line_spacing={} \
         top_spacing={} bottom_spacing={} styled_spans={}",
        text_data.text, alignment, word_wrap, fixed_line_space,
        section1_data.width, section1_data.height, section1_data.page_height,
        paragraph_info.line_height, paragraph_info.line_count,
        paragraph_info.left_indent, paragraph_info.right_indent, paragraph_info.first_indent,
        paragraph_info.line_spacing, paragraph_info.top_spacing, paragraph_info.bottom_spacing,
        styled_spans.len(),
    );

    // Compute member-level default font_size from the two char run
    // sections' first runs. Director's `the fontSize of member` returns
    // the SMALLER of:
    //   - 0x0004's first run's referenced style's font_size, and
    //   - 0x0005's first run's referenced style's font_size
    //
    // Rationale: 0x0004 (Paige's style_run_key) is the per-character style
    // run array; 0x0005 (par_run_key in Paige but reused here) is a
    // separate per-character style table that supplies the *member-level*
    // default. When both reference different styles for offset 0,
    // Director picks the smaller — empirically:
    //   - Member 71: 0x0004→10, 0x0005→12  → 10 ✓
    //   - Member 74: 0x0004→15, 0x0005→12  → 12 ✓
    //   - Member 82: Section 7 declares 0 styles (member.html was set
    //     without a `size` attr, so no style runs are authored). Our
    //     parser still reads stray bytes past the declared count, so we
    //     gate the lookup on `declared_style_count`: when 0, both
    //     lookups fail → consumer falls back to engine default 12. ✓
    //
    // (We tried "use 0x0004 only when its font_index is valid" first, but
    // our Section 9 parser is more permissive than the C# reference and
    // marks word0=14 as valid even when only Arial+Verdana are authored,
    // breaking member 74. The min rule sidesteps that question.)
    // If Section 7 declared zero styles (member 82 — text was set via
    // member.html with no `size=` attr, so no style runs are authored),
    // no font size can come from the styles regardless of what stray
    // bytes our packer pulled past the count. Fall through to consumer
    // defaults.
    // For HTML-set members, Director re-maps the stored XMED font_size
    // through its `<font size=N>` table — stored values are slightly
    // compressed compared to the HTML pt sizes Director reports
    // (size=5 → stored 16, returned 18; size=4 → stored 13, returned 14;
    // size=6 → stored 19, returned 24). The mapping is "round up to next
    // entry in [8,10,12,14,18,24,36]".
    //
    // The detector for "HTML-set" is `font_index_valid` on the style
    // 0x0004's first run references. Director's HTML-set members have a
    // legit `<font face="...">` tag whose font index points into a real
    // entry in Section 9. Plain styled-text members (set via member.text
    // or programmatic font assignment) leave the per-style font_index as
    // a synthesised value beyond Section 9's range, which our parser
    // marks `font_index_valid=false`.
    //
    // Verified empirically:
    //   - Member 30 (HTML "directions"): style[7] valid → 16 → 18 ✓
    //   - Member 8  (HTML rich text):    style[5] valid → 19 → 24 ✓
    //   - Member 71 (HTML small text):   style[3] valid → 10 → 10 ✓
    //   - Member 74 (#fixed clock):      style[4] INVALID (word0=14 vs
    //     2 fonts in Section 9) → fall through to 0x0005 first → 12 ✓
    //   - Member 82 (HTML, no size attr): declared_count=0 → None → 12 ✓
    let declared_count = style_data.declared_style_count;
    let lookup_size_via = |runs: &[CharRun]| -> Option<u16> {
        if declared_count == 0 {
            return None;
        }
        let run = runs.first()?;
        let style = style_data.styles.get(run.style_index as usize)?;
        if style.font_size == 0 {
            return None;
        }
        Some(style.font_size)
    };
    // member.fontSize sources style.font_size directly. Since the parser
    // applies Paige's `dword84` (real `point`) on every style, the first
    // char run's style carries the correct authored point size for both
    // Director-text and HTML members — no HTML size-class rounding is
    // needed. The previous detector ("word48 != 0 → HTML, round up via
    // HTML_PT") false-positived on Director-text members where word48 is
    // also non-zero, e.g. member 35 (Verdana 9pt italic, word48=2): the
    // 9pt point size got rounded up to 10pt via HTML_PT[1].
    //
    // Prefer Section 0x0004 (style runs — the authoritative char-run table
    // pointing into Section 7 styles). Fall back to Section 0x0005 only
    // when 0x0004 is absent, empty, or its first run points to an invalid
    // style (e.g. member 74 FurniFactory clock display: style[4] INVALID
    // → s4=None → use s5=12). Worldbuilder member 290 'Guard Tower':
    // s4=25 (style[6]=Arial Black 25), s5=12 (par_info table reused as
    // style index, points to style[0]=Arial 12). The earlier `min` rule
    // mis-picked the smaller value for member 290 — Director returns 25.
    let s4 = lookup_size_via(&section4_char_runs);
    let s5 = lookup_size_via(&section5_char_runs);
    let default_font_size = s4.or(s5);

    // When Section 7 declares 0 styles AND no char-run path produced a
    // size, fall back to the HTML `<font size=N>` attribute Director
    // captures in Section 1 at index [42] (verified empirically by
    // diffing member 36 vs member 82's S1 dumps — only member 36 had
    // the attribute set, matching its HTML `<font size=6>` tag → 24 pt).
    // Non-HTML-set or default-sized members store 0 there and fall
    // through to the engine default (12).
    let default_font_size = default_font_size.or_else(|| {
        let attr = section1_data.html_font_size_attr;
        if (1..=7).contains(&attr) {
            const HTML_PT: [u16; 7] = [8, 10, 12, 14, 18, 24, 36];
            Some(HTML_PT[(attr - 1) as usize])
        } else {
            None
        }
    });

    // Build par_runs from Section 0x0005's char-run-format entries.
    // Same wire format (position, index) but the index points into the
    // par_infos table (0x0007), NOT the styles table (0x0006). Used by
    // the `member.line[N].fixedLineSpace` getter and the renderer's
    // per-line stride lookup.
    let par_runs: Vec<ParRun> = section5_char_runs
        .iter()
        .map(|cr| ParRun {
            position: cr.position,
            par_info_index: cr.style_index,
        })
        .collect();

    Ok(XmedStyledText {
        text: text_data.text,
        styled_spans,
        alignment,
        word_wrap,
        fixed_line_space,
        width: section1_data.width,             // From Section 1 dword48
        height: section1_data.height,           // From Section 1 dword8C
        page_height: section1_data.page_height, // From Section 1 dword90
        line_height: paragraph_info.line_height,
        line_count: paragraph_info.line_count,
        left_indent: paragraph_info.left_indent,
        right_indent: paragraph_info.right_indent,
        first_indent: paragraph_info.first_indent,
        line_spacing: paragraph_info.line_spacing,
        top_spacing: paragraph_info.top_spacing,
        bottom_spacing: paragraph_info.bottom_spacing,
        par_infos,
        par_runs,
        bg_color: section1_data.bg_color,
        default_font_size,
    })
}

/// Parse Section 3 - Text Content
/// Format: 00 [length], [text] 03
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
    // Use Mac Roman decoding for bytes 0x80-0xFF (Director files use Mac Roman encoding)
    let mut text = String::new();
    for i in text_start..text_end {
        let ch = mac_roman_to_char(data[i]);
        if ch == '\0' {
            break; // Stop at first null byte (padding)
        }
        text.push(ch);
    }

    debug!("    Section 3: {} chars", text.len());

    Ok(Section3Data { text })
}

/// Parse Section 1 - Document Header
/// Extracts document version, width, height, and pageHeight
fn parse_section_1(data: &[u8]) -> Result<Section1Data, String> {
    if data.is_empty() {
        return Ok(Section1Data::default());
    }

    let mut packer = Packer::new(data.to_vec());
    let mut section1 = Section1Data::default();

    // Debug: show raw bytes
    let preview_len = data.len().min(20);
    let hex_preview: Vec<String> = data[0..preview_len].iter().map(|b| format!("{:02X}", b)).collect();
    debug!("Section 1 parse: {} bytes, first {}: {}",
                                     data.len(), preview_len, hex_preview.join(" "));

    // 1. int4 (doc version) - line 851
    section1.doc_version = packer.unpack_num();
    debug!("    Section1: doc_version={}", section1.doc_version);

    // 2. dwordC - line 857
    if packer.remaining() >= 2 { packer.unpack_num(); }

    // 3. dword14 value - line 860
    if packer.remaining() >= 2 { packer.unpack_num(); }

    // 4. if (int4 >= 65547) dword18 - lines 864-868
    if section1.doc_version >= 65547 && packer.remaining() >= 2 {
        packer.unpack_num();
    }

    // 5. if (int4 >= 65552) - lines 871-885
    if section1.doc_version >= 65552 {
        // dword20
        if packer.remaining() >= 2 { packer.unpack_num(); }
        // dword418
        if packer.remaining() >= 2 { packer.unpack_num(); }
        // dword41C
        if packer.remaining() >= 2 { packer.unpack_num(); }
    }

    // 6. dwordB0 - line 888
    if packer.remaining() >= 2 { packer.unpack_num(); }

    // 7. dword48 (width) - line 889
    if packer.remaining() >= 2 {
        section1.width = packer.unpack_num();
        debug!("    Section1: width (dword48)={}", section1.width);
    }

    // 8. dword8C (fallback height) - line 890
    if packer.remaining() >= 2 {
        section1.height = packer.unpack_num();
        debug!("    Section1: height (dword8C)={}", section1.height);
    }

    // 9. dword90 (pageHeight) - line 891
    if packer.remaining() >= 2 {
        section1.page_height = packer.unpack_num();
        debug!("    Section1: pageHeight (dword90)={}", section1.page_height);
    }

    // Values [11..29] - skip 19 intermediate values to reach bg_color at [30-32]
    for _ in 11..30 {
        if packer.remaining() >= 2 {
            packer.unpack_num();
        }
    }

    // Values [30-32] - background color as 16-bit Director color components
    // High byte of each value is the actual 8-bit color (e.g. 0xCC00 -> 0xCC)
    if packer.remaining() >= 2 {
        let bg_r = packer.unpack_num();
        if packer.remaining() >= 2 {
            let bg_g = packer.unpack_num();
            if packer.remaining() >= 2 {
                let bg_b = packer.unpack_num();
                let r = ((bg_r >> 8) & 0xFF) as u8;
                let g = ((bg_g >> 8) & 0xFF) as u8;
                let b = ((bg_b >> 8) & 0xFF) as u8;
                section1.bg_color = Some((r, g, b));
                debug!("    Section1: bg_color=({}, {}, {}) from raw ({:#06X}, {:#06X}, {:#06X})",
                    r, g, b, bg_r, bg_g, bg_b);
            }
        }
    }

    // Values [33..42] — read 10 more values to reach the HTML font-size
    // attribute at index [42] (verified against the C# reference parser's
    // S1-DUMP indices). Capture the last read (index [42]); intermediates
    // are discarded but still consume packer state.
    for i in 33..=42 {
        if packer.remaining() < 2 {
            break;
        }
        let val = packer.unpack_num();
        if i == 42 {
            section1.html_font_size_attr = val;
            debug!("    Section1: html_font_size_attr (idx 42)={}", val);
        }
    }

    debug!("Section 1: version={}, width={}, height={}, pageHeight={}, bg_color={:?}, html_size={}",
        section1.doc_version, section1.width, section1.height, section1.page_height,
        section1.bg_color, section1.html_font_size_attr);

    Ok(section1)
}

/// Parse Section 6 - Character Runs
/// Each run defines which style applies to text starting at a position
fn parse_section_6(data: &[u8]) -> Result<Section6Data, String> {
    let mut packer = Packer::new(data.to_vec());
    let mut char_runs = Vec::new();

    // Read character runs using Packer encoding
    while packer.remaining() > 0 {
        if packer.remaining() < 2 {
            break; // Not enough data for a full run
        }

        let position = packer.unpack_num() as u32;

        if packer.remaining() < 2 {
            break; // Not enough data for style index
        }

        let style_index = packer.unpack_num() as u16;

        debug!("    CharRun: position={}, style_index={}", position, style_index);

        char_runs.push(CharRun {
            position,
            style_index,
        });
    }

    debug!("    Section 6: {} character runs", char_runs.len());
    debug!(" Section 6: Parsed {} character runs", char_runs.len());

    Ok(Section6Data { char_runs })
}

/// Packer for unpacking variable-length encoded data
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
    fn unpack_num(&mut self) -> i32 {
        self.unpack_num_debug(false)
    }

    /// Debug version of unpack_num that can log details
    fn unpack_num_debug(&mut self, debug: bool) -> i32 {
        // Handle repeat mode
        if self.repeat_count > 0 {
            self.repeat_count -= 1;
            if debug {
                debug!("    [Packer] Repeat mode, returning last_value={}", self.last_value);
            }
            return self.last_value;
        }

        if self.pos >= self.data.len() {
            if debug {
                debug!("    [Packer] pos >= data.len(), returning 0");
            }
            return 0;
        }

        let ctrl = self.data[self.pos];
        if debug {
            debug!("    [Packer] pos={}, ctrl=0x{:02X} ('{}')",
                self.pos, ctrl, if ctrl >= 0x20 && ctrl < 0x7F { ctrl as char } else { '?' });
        }
        self.pos += 1;

        let mut val: i32 = 0;

        // Check for repeat mode (bit 7 set)
        if (ctrl & 0x80) != 0 {
            val = self.last_value;
            if debug {
                debug!("    [Packer] Bit 7 set, repeat mode, val={}", val);
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
                    debug!("    [Packer] Hex string: '{}' ({} chars)", hex_str, hex_str.len());
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
                        debug!("    [Packer] Failed to parse hex '{}'", hex_str_clean);
                    }
                    val = 0;
                }
            } else {
                if debug {
                    debug!("    [Packer] No hex digits found after ctrl byte");
                }
            }

            // Handle type code for short values
            let type_code = ctrl & 0x0F;
            if type_code == 1 {
                val = (val as u16) as i32;
                if debug {
                    debug!("    [Packer] type_code=1, converted to u16: {}", val);
                }
            }
        }

        self.last_value = val;
        val
    }

    /// UnpackRefcon
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

                debug!("    UnpackRefcon (PtrBytes): size={}, consumed {} bytes", size, bytes_consumed);
                return bytes_consumed as i32;
            }
        }

        // Default case: just read one number
        self.unpack_num()
    }
}

/// Parse Section 7 - Style Definitions
/// Uses Packer to extract variable-length encoded style data
fn parse_section_7(data: &[u8], font_names: &[String], doc_version: i32) -> Result<Section7Data, String> {
    let mut packer = Packer::new(data.to_vec());
    let mut styles: Vec<XmedStyle> = Vec::new();

    // Log raw first bytes for debugging color/style issues
    let preview_len = data.len().min(40);
    let hex_preview: Vec<String> = data[0..preview_len].iter().map(|b| format!("{:02X}", b)).collect();
    debug!(" Section 7 (styles): {} bytes, first {}: {}",
                                     data.len(), preview_len, hex_preview.join(" "));
    // Read style count
    let style_count = packer.unpack_num();

    // style_count is a hint but the packed data often contains more styles than declared.
    // Character runs (Section 0x0004) can reference styles beyond style_count, so we must
    // parse all styles present in the packed data, not just the declared count.
    debug!(" Section 7: count={}, doc_version={}, remaining={}", style_count, doc_version, packer.remaining());

    let mut style_idx = 0;
    while style_idx < 100 && packer.remaining() > 4 {
        let mut style = XmedStyle::default();
        let mut parse_failed = false;

        debug!("    Style {}: pos={}, remaining={} bytes",
                                         style_idx, packer.pos, packer.remaining());

        if !parse_failed && packer.remaining() >= 2 {
            let font_index = packer.unpack_num();
            style.font_index_raw = font_index;
            if font_index >= 0 && (font_index as usize) < font_names.len() {
                style.font_name = font_names[font_index as usize].clone();
                style.font_index_valid = true;
            } else if let Some(prev) = styles.last() {
                // Some files emit indices beyond parsed section-9 entries; inherit prior font.
                // Mark `font_index_valid=false` so member-level lookups can
                // tell this style's font reference was synthesised, not
                // authored. Director appears to skip such styles when
                // computing `member.fontSize` and falls back to the next
                // char run array.
                style.font_name = prev.font_name.clone();
                style.font_index_valid = false;
            }
            debug!("    Style {}: word0={} -> font='{}' valid={}",
                                             style_idx, font_index, style.font_name,
                                             style.font_index_valid);
        } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 {
            let font_size = packer.unpack_num();
            if font_size > 0 && font_size <= 200 {
                style.font_size = font_size as u16;
            }
            debug!("    Style {}: word46 (fontSize)={}", style_idx, font_size);
        } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 {
            let word_wrap_value = packer.unpack_num();
            // 0 = #adjust (no wrap), 1 = #scroll, 2 = #fixed, 3 = #limit (all wrap)
            // ALSO: when this style came from an HTML `<font size=N>` tag,
            // this slot stores the per-style HTML size class (non-zero);
            // when the wrapping `<font>` had no `size=` attribute, it's 0.
            // We keep the original `word_wrap` interpretation but also
            // expose the raw value for the member.fontSize logic.
            style.word_wrap = word_wrap_value != 0;
            style.word48_raw = word_wrap_value;
            debug!("    Style {}: word48_raw={} → word_wrap={}", style_idx, word_wrap_value, style.word_wrap);
        } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        if !parse_failed && packer.remaining() >= 2 { packer.unpack_num(); } else { parse_failed = true; }

        // 10-11. pgUnpackColor (foreColor) - line 1295 (4 values)
        // Color format: c1=R, c2=G, c3=B, c4=unused
        // Each 16-bit value has the actual 8-bit color in the high byte (e.g., 0x9900 for R=0x99)
        let mut color_values = Vec::new();
        for _ in 0..4u8 {
            if !parse_failed && packer.remaining() >= 2 {
                color_values.push(packer.unpack_num());
            } else { parse_failed = true; break; }
        }
        if color_values.len() >= 4 {
            let c1 = color_values[0] as u32;
            let c2 = color_values[1] as u32;
            let c3 = color_values[2] as u32;
            let _c4 = color_values[3] as u32;
            style.fore_color = (0xFF << 24) |
                               ((c1 >> 8) << 16) |
                               ((c2 >> 8) << 8) |
                               (c3 >> 8);
            style.color = Some(style.fore_color);
        }

        // 12-13. pgUnpackColor (backColor) - line 1296 (4 values)
        // Color format: c1=R, c2=G, c3=B, c4=unused
        let mut back_color_values = Vec::new();
        for _ in 0..4u8 {
            if !parse_failed && packer.remaining() >= 2 {
                back_color_values.push(packer.unpack_num());
            } else { parse_failed = true; break; }
        }
        if back_color_values.len() >= 4 {
            let c1 = back_color_values[0] as u32;
            let c2 = back_color_values[1] as u32;
            let c3 = back_color_values[2] as u32;
            let _c4 = back_color_values[3] as u32;
            style.back_color = (0xFF << 24) |
                               ((c1 >> 8) << 16) |
                               ((c2 >> 8) << 8) |
                               (c3 >> 8);
        }

        debug!("    Style {}: foreColor=0x{:08X}, backColor=0x{:08X}",
                                         style_idx, style.fore_color, style.back_color);

        // 14. if (int4 < 65547) dword78 - line 1299-1302
        if !parse_failed && doc_version < 65547 && packer.remaining() >= 2 {
            packer.unpack_num();
        }

        // 15-25. dword80 through dwordA8 (11 values) - lines 1304-1314
        // Paige style_info pg_fixed block (each = 16.16 fixed-point):
        //   index 0 = char_width
        //   index 1 = point   ← REAL fontSize (Director's `the fontSize`)
        //   index 2 = left_overhang
        //   index 3 = right_overhang
        //   index 4 = top_extra
        //   index 5 = bot_extra
        //   index 6 = space_extra (kerning)
        //   index 7 = char_extra (charSpacing)
        //   index 8/9/10 = trailing pg_fixed slots
        //
        // We previously sourced font_size from `word46`, which is actually
        // Paige's `style_num` (RTF stylesheet ID) — close to the real point
        // value for some styles but wrong for most. Junkbot help member 124
        // S7.7 reports word46=5 but Director queries fontSize=6; the real
        // value lives in `dword84` (pg_fixed point) where 393216/65536=6.
        // Override style.font_size with the dword84 value when it's set.
        let mut paige_slots: Vec<i32> = Vec::with_capacity(11);
        for i in 0..11 {
            if !parse_failed && packer.remaining() >= 2 {
                let value = packer.unpack_num();
                paige_slots.push(value);
                // dword84 is at index 1 — Paige's `point` (font size in
                // 16.16 fixed-point). Trust it over word46.
                if i == 1 {
                    let real_size = (value as i64) / 65536;
                    if real_size > 0 && real_size <= 200 {
                        style.font_size = real_size as u16;
                        debug!("    Style {}: dword84 (real fontSize)={} (raw={})",
                                                         style_idx, real_size, value);
                    }
                }
                // dword98 is at index 6, contains kerning as fixed-point (value * 65536)
                if i == 6 {
                    style.kerning = value / 65536;  // Convert from fixed-point
                    debug!("    Style {}: dword98 (kerning)={} (raw={})",
                                                     style_idx, style.kerning, value);
                }
                // dword9C is at index 7, contains charSpacing as fixed-point (value * 65536)
                if i == 7 {
                    style.char_spacing = value / 65536;  // Convert from fixed-point
                    debug!("    Style {}: dword9C (charSpacing)={} (raw={})",
                                                     style_idx, style.char_spacing, value);
                }
            } else { parse_failed = true; }
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
        // For version 262145 >= 257, v3 = 0, so count = 32
        let gap2_count = if doc_version >= 257 { 32 } else { 32 - 16 }; // v3 & 0xF0 = -16 when v3=-1

        if !parse_failed {
            debug!("    Style {}: Reading gap2 ({} values), remaining {} bytes",
                                             style_idx, gap2_count, packer.remaining());
            let mut gap2 = Vec::new();
            for i in 0..gap2_count {
                if packer.remaining() >= 2 {
                    gap2.push(packer.unpack_num());
                } else {
                    debug!("    Style {}: Ran out at gap2[{}]", style_idx, i);
                    break;
                }
            }

            if gap2.len() >= 3 {
                // Paige's `style_info.styles[]` array (PAIGE.H:419-449) uses
                // any non-zero short to mean "set". Empirically, Director text
                // members written via the dialog use `1` while HTML imports
                // (and pgFillStylesFromLogFont) use `-1` (= 65535). Either is
                // truthy, so the test must be `!= 0` not `== 1` — the latter
                // silently drops bold/italic/underline on every HTML member.
                style.bold = gap2[0] != 0;
                style.italic = gap2[1] != 0;
                style.underline = gap2[2] != 0;
                debug!("    Style {}: gap2[0-2]=[{},{},{}] -> bold={}, italic={}, underline={}",
                                                 style_idx, gap2[0], gap2[1], gap2[2],
                                                 style.bold, style.italic, style.underline);
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

        debug!("    Style {}: FINAL -> font='{}', size={}, bold={}, italic={}, underline={}",
                                         style_idx, style.font_name, style.font_size,
                                         style.bold, style.italic, style.underline);

        if parse_failed {
            debug!("    Style {}: parse failed, stopping", style_idx);
            break;
        }
        styles.push(style);
        style_idx += 1;
    }

    debug!("   Section 7: Parsed {} style(s) (initial count was {})", styles.len(), style_count);

    let declared_style_count = if style_count > 0 { style_count as usize } else { 0 };

    // If the section header declared zero styles, no font_size info is
    // authored — every style we parsed came from stray bytes past the
    // declared count and Director treats them all as untrusted. Zero the
    // sizes here so downstream span construction and member-level font
    // size lookups all default to engine fallbacks. (When declared_count
    // > 0, leave parsed styles alone: char runs in some files
    // legitimately reference styles past the declared count, and zeroing
    // those would regress members whose first run uses such a style.)
    if declared_style_count == 0 {
        for s in styles.iter_mut() {
            s.font_size = 0;
        }
    }

    if styles.is_empty() {
        styles.push(XmedStyle::default());
    }

    debug!("    Section 7: Parsed {} style(s), declared={}", styles.len(), declared_style_count);

    Ok(Section7Data { styles, declared_style_count })
}

/// Paragraph formatting values from Section 8
#[derive(Debug, Clone)]
struct ParagraphInfo {
    alignment: TextAlignment,
    // Field properties (from Section 8[0])
    line_height: i32,      // word2
    line_count: i32,       // dword274
    // Paragraph formatting
    left_indent: i32,
    right_indent: i32,
    first_indent: i32,
    line_spacing: i32,
    top_spacing: i32,
    bottom_spacing: i32,
}

impl Default for ParagraphInfo {
    fn default() -> Self {
        ParagraphInfo {
            alignment: TextAlignment::Left,
            line_height: 0,
            line_count: 0,
            left_indent: 0,
            right_indent: 0,
            first_indent: 0,
            line_spacing: 0,
            top_spacing: 0,
            bottom_spacing: 0,
        }
    }
}

/// Parse Section 8 - Paragraph Runs (Alignment and paragraph formatting)
/// - Alignment: Left (default), Center (byte[36]=0x31), Right (0x32), Justify (0x33)
/// - Paragraph formatting: int25C[0-2], dword270, dword278, dword27C
fn parse_section_8(data: &[u8], doc_version: i32) -> Result<ParagraphInfo, String> {
    debug!("Parsing paragraph runs: {} bytes", data.len());

    let mut info = ParagraphInfo::default();

    // Left-aligned: 36 bytes or less (single paragraph run, alignment implicit=0)
    if data.len() <= 36 {
        debug!("Alignment: Left (section size {} <= 36)", data.len());
        info.alignment = TextAlignment::Left;
        return Ok(info);
    }

    // Center/Right/Justify: more than 36 bytes with alignment at byte[36]
    let alignment_byte = data[36];
    info.alignment = match alignment_byte {
        0x01 | 0x31 => {
            debug!("  Paragraph alignment: Center (byte[36]=0x{:02X})", alignment_byte);
            TextAlignment::Center
        }
        0x02 | 0x32 => {
            debug!("  Paragraph alignment: Right (byte[36]=0x{:02X})", alignment_byte);
            TextAlignment::Right
        }
        0x03 | 0x33 => {
            debug!("  Paragraph alignment: Justify (byte[36]=0x{:02X})", alignment_byte);
            TextAlignment::Justify
        }
        _ => {
            debug!("  Paragraph alignment: Left (byte[36]=0x{:02X} unknown)", alignment_byte);
            TextAlignment::Left
        }
    };

    // Parse paragraph formatting from packed data
    // The formatting is in the second paragraph run (if present)
    // - int25C[0-2] = LeftIndent, RightIndent, FirstIndent
    // - dword270 = lineSpacing
    // - dword278 = TopSpacing
    // - dword27C = BottomSpacing
    let mut packer = Packer::new(data.to_vec());

    // Decode all packed values to find the paragraph formatting
    // and the formatting values are at these relative positions:
    // - int25C[0-2] at indices 33-35 within the paragraph
    // - dword270 at index ~37 within the paragraph
    // - dword278 at index ~39 within the paragraph
    // - dword27C at index ~40 within the paragraph

    let mut values: Vec<i32> = Vec::new();
    while packer.remaining() > 0 {
        values.push(packer.unpack_num());
        if values.len() > 200 {
            break; // Safety limit
        }
    }

    debug!("Decoded {} packed values", values.len());

    // Extract field properties from first paragraph run (Section 8[0])
    // Note: pageHeight comes from Section 1 dword90, not Section 8
    // - word2 = lineHeight (index 1)
    if values.len() >= 2 {
        info.line_height = values[1];
        debug!(
            "Field properties: lineHeight={}",
            info.line_height
        );
    }

    // Look for paragraph formatting values in the decoded stream
    // For a file with 92 bytes (like getProp02.txt):
    // - Index 57: LeftIndent (36)
    // - Index 58: RightIndent (43)
    // - Index 59: FirstIndent (50)
    // - Index 62: lineSpacing (19)
    // - Index 64: TopSpacing (2)
    // - Index 65: BottomSpacing (8)

    // Try to extract values if we have enough data
    if values.len() > 65 {
        // Check if the values look like formatting (reasonable ranges)
        let idx_left = 57;
        let idx_right = 58;
        let idx_first = 59;
        let idx_line = 62;
        let idx_top = 64;
        let idx_bottom = 65;

        // Validate the values are in reasonable ranges for paragraph formatting
        let left = values.get(idx_left).copied().unwrap_or(0);
        let right = values.get(idx_right).copied().unwrap_or(0);
        let first = values.get(idx_first).copied().unwrap_or(0);
        let line = values.get(idx_line).copied().unwrap_or(0);
        let top = values.get(idx_top).copied().unwrap_or(0);
        let bottom = values.get(idx_bottom).copied().unwrap_or(0);

        // Only use values if they look like reasonable formatting values
        // (not garbage from other data structures)
        if left >= 0 && left <= 1000 && right >= 0 && right <= 1000 {
            info.left_indent = left;
            info.right_indent = right;
            info.first_indent = first;
            info.line_spacing = line;
            info.top_spacing = top;
            info.bottom_spacing = bottom;

            debug!(
                "Paragraph formatting: LeftIndent={}, RightIndent={}, FirstIndent={}, LineSpacing={}, TopSpacing={}, BottomSpacing={}",
                info.left_indent, info.right_indent, info.first_indent,
                info.line_spacing, info.top_spacing, info.bottom_spacing
            );
        }
    }

    Ok(info)
}

/// Parse Section 0x0007 (called Section 8 by C# convention) into a list of
/// per-paragraph `par_info` records. Director uses these via Section 0x0005
/// (par_run_key) to assign each text range a different `line_spacing`.
/// Faithful port of `Sections.cs::ProcessSection8` field-by-field; reads
/// entries until the packer is exhausted (was hardcoded to 2 in C#).
fn parse_section_8_par_infos(data: &[u8], doc_version: i32) -> Vec<ParInfo> {
    let mut packer = Packer::new(data.to_vec());
    let mut par_infos: Vec<ParInfo> = Vec::new();
    while packer.remaining() > 4 && par_infos.len() < 32 {
        let start_pos = packer.pos;
        let mut info = ParInfo::default();

        // word0 = Paige par_info.justification (0=left, 1=center, 2=right, 3=justify)
        info.justification = packer.unpack_num();
        // word2 (lineHeight)
        info.line_height = packer.unpack_num();
        // dword250 (boxType) — discard
        packer.unpack_num();
        // int25C[3] — LeftIndent, RightIndent, FirstIndent
        info.left_indent = packer.unpack_num();
        info.right_indent = packer.unpack_num();
        info.first_indent = packer.unpack_num();
        // dword268 (border), dword26C (margin) — discard
        packer.unpack_num();
        packer.unpack_num();
        // dword270 (LineSpacing) ← target field
        info.line_spacing = packer.unpack_num();
        // dword274 if version >= 65547
        if doc_version >= 65547 { packer.unpack_num(); }
        // dword278..dword294 (8 values)
        for _ in 0..8 { packer.unpack_num(); }
        // wordC if version >= 65552
        if doc_version >= 65552 { packer.unpack_num(); }
        // UnpackRefcon (variable)
        packer.unpack_refcon(doc_version);
        // dword2E8
        packer.unpack_num();
        // gap2A8 (8 values)
        for _ in 0..8 { packer.unpack_num(); }
        // wordE: extra inner loop count
        let word_e = packer.unpack_num();
        let sizea = word_e;
        if word_e > 0 {
            let actual = if word_e > 32 { 32 } else { word_e };
            for _ in 0..actual {
                // 3 UnpackNum + 1 UnpackRefcon
                packer.unpack_num();
                packer.unpack_num();
                packer.unpack_num();
                packer.unpack_refcon(doc_version);
            }
            if sizea > 32 {
                for _ in 0..(sizea - 32) {
                    packer.unpack_num();
                }
            }
        }
        // dword258 if version >= 8
        if doc_version >= 8 { packer.unpack_num(); }
        // word4 (FirstIndent) if version >= 65548
        if doc_version >= 65548 { packer.unpack_num(); }
        // dword298, word6 (TopSpacing), word8 (BottomSpacing), wordA if version >= 65552
        if doc_version >= 65552 {
            packer.unpack_num();
            info.top_spacing = packer.unpack_num();
            info.bottom_spacing = packer.unpack_num();
            packer.unpack_num();
        }
        // dword254 if version >= 65555
        if doc_version >= 65555 { packer.unpack_num(); }
        // dword210..238 (9 values) if version >= 131075
        if doc_version >= 131075 {
            for _ in 0..9 { packer.unpack_num(); }
        }
        // dword29C if version >= 131090
        if doc_version >= 131090 { packer.unpack_num(); }
        // dword244, 240, 24C, 248 if version >= 131090
        if doc_version >= 131090 {
            packer.unpack_num();
            packer.unpack_num();
            packer.unpack_num();
            packer.unpack_num();
            // dword218 if version >= 196614
            if doc_version >= 196614 { packer.unpack_num(); }
            // dword234 — extra read in the >=196615 branch
            if doc_version >= 196615 { packer.unpack_num(); }
            // dword23C if version >= 196616
            if doc_version >= 196616 { packer.unpack_num(); }
        }

        // Safety: if no progress made, abort to avoid infinite loop.
        if packer.pos == start_pos { break; }

        debug!(
            "  par_info[{}] line_spacing={} top_spacing={} bottom_spacing={}",
            par_infos.len(), info.line_spacing, info.top_spacing, info.bottom_spacing
        );
        par_infos.push(info);
    }
    par_infos
}

/// Font information from Section 9
#[derive(Debug, Clone)]
struct FontInfo {
    name: String,
    kerning: bool,
    anti_alias: bool,
}

/// Parse Section 9 - Font Definitions
/// Font names stored using PgUnpackPtrBytes format: 00 [hex_size] [font_name_bytes]
fn parse_section_9(data: &[u8], doc_version: i32) -> Result<Vec<String>, String> {
    debug!("  Parsing Section 9 (Font Definitions): {} bytes", data.len());

    let mut font_names = Vec::new();
    let mut font_infos = Vec::new();
    let mut offset = 0;

    // Check if we have font names (starts with 0x00 marker)
    if data.is_empty() || data[0] != 0x00 {
        debug!("    Section 9: No font names (no 0x00 marker), using default");
        return Ok(vec!["Arial".to_string()]);
    }

    // Each font ENTRY has:
    //   - First font name (64 bytes: 00 + hex_size + comma + data)
    //   - Second font name (64 bytes, usually empty)
    //   - Properties (Packer-encoded, ~38 bytes)
    // Total per entry: ~174 bytes
    // Keep reading entries until data runs out (some members have 3+ font entries).
    let mut entry_idx = 0;
    loop {
        if offset >= data.len() || data[offset] != 0x00 {
            break;
        }

        debug!(" Font Entry {}: Starting at offset {}", entry_idx, offset);

        // Read FIRST font name for this entry
        match read_font_name(data, &mut offset, entry_idx, 0) {
            Ok(Some(name)) => {
                debug!("    Entry {}, Name 1: '{}' at offset {}", entry_idx, name, offset);
                font_names.push(name);
            }
            Ok(None) => {
                debug!("    Entry {}, Name 1: (empty)", entry_idx);
            }
            Err(e) => {
                debug!("    Entry {}: Error reading first name: {}", entry_idx, e);
                break;
            }
        }

        // Read SECOND font name for this entry (usually empty)
        if doc_version >= 65550 {
            match read_font_name(data, &mut offset, entry_idx, 1) {
                Ok(Some(name)) => {
                    if !name.is_empty() {
                        debug!("    Entry {}, Name 2: '{}' (unusual - second name not empty!)", entry_idx, name);
                        font_names.push(name);
                    }
                }
                Ok(None) => {
                    // Expected - second name is usually empty
                }
                Err(e) => {
                    debug!("    Entry {}: Error reading second name: {}", entry_idx, e);
                    break;
                }
            }
        }

        // Read properties section by parsing with Packer to advance offset correctly
        let (kerning, anti_alias) = match read_font_properties(data, &mut offset, entry_idx, doc_version) {
            Ok((font_style, anti_alias_val, kerning_val)) => {
                // Properties read successfully, offset now points to next entry
                // Extract boolean values
                let kerning = kerning_val > 0;
                let anti_alias = anti_alias_val > 0;

                debug!("    Entry {}: kerning={}, antiAlias={}",
                                                 entry_idx, kerning, anti_alias);
                (kerning, anti_alias)
            }
            Err(e) => {
                debug!("    Entry {}: Error reading properties: {}, stopping", entry_idx, e);
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
        entry_idx += 1;
    }

    debug!("[Section9] Parsed {} entries, font_names={:?}", entry_idx, font_names);

    if font_names.is_empty() {
        debug!("    Section 9: No fonts parsed, using default 'Arial'");
        font_names.push("Arial".to_string());
    }

    debug!("   Section 9: Parsed {} font name(s) with properties", font_names.len());

    // For now, just return font names for compatibility with Section 7 parsing
    // TODO: Expose kerning and anti_alias properties in XmedStyledText or StyledSpan
    Ok(font_names)
}

/// Read font properties section and advance offset
/// Returns (font_style, anti_alias, kerning)
fn read_font_properties(data: &[u8], offset: &mut usize, entry_idx: usize, doc_version: i32) -> Result<(u16, u16, u16), String> {
    if *offset >= data.len() {
        return Err(format!("Offset {} beyond data length {}", offset, data.len()));
    }

    let start_offset = *offset;

    // Create a Packer starting at current offset
    let remaining_data = data[*offset..].to_vec();
    let mut packer = Packer::new(remaining_data);

    debug!("    Entry {}: Reading properties, doc_version={}", entry_idx, doc_version);

    let word80 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    let word82 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    let word84 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    let word88 = if packer.remaining() >= 2 { packer.unpack_num() as u16 } else { 0 };

    let word8a = if doc_version >= 65552 && packer.remaining() >= 2 {
        packer.unpack_num() as u16
    } else {
        0
    };

    let _dword90 = if packer.remaining() >= 2 { packer.unpack_num() as u32 } else { 0 };

    if packer.remaining() >= 2 {
        packer.unpack_refcon(doc_version);
    }

    for i in 0..8 {
        if packer.remaining() >= 2 {
            packer.unpack_num();
        } else {
            debug!("    Entry {}: Ran out at word94[{}]", entry_idx, i);
            break;
        }
    }

    if doc_version >= 256 {
        if packer.remaining() >= 2 { packer.unpack_num(); } // dword8C
        if packer.remaining() >= 2 { packer.unpack_num(); } // word86
    }

    // Advance offset by how much the packer consumed
    let consumed = packer.pos;
    *offset += consumed;

    debug!("    Entry {}: fontStyle=0x{:04X}, fontSize={}, kerning=0x{:04X}, antiAlias=0x{:04X}, consumed {} bytes",
                                     entry_idx, word80, word82, word88, word8a, consumed);

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

    // Skip comma if present
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

    let deduped_runs = normalize_char_runs(&char_runs.char_runs);

    // Build a char-index-to-byte-index map for safe slicing.
    // run.position values are character indices (one per original byte in Mac Roman),
    // but after decoding to UTF-8 some characters are multi-byte.
    let char_byte_offsets: Vec<usize> = text.char_indices().map(|(i, _)| i).collect();
    let char_count = char_byte_offsets.len();

    // Create spans for each run
    for (i, run) in deduped_runs.iter().enumerate() {
        let char_start = run.position as usize;
        let char_end = if i + 1 < deduped_runs.len() {
            deduped_runs[i + 1].position as usize
        } else {
            char_count
        };

        if char_start >= char_count {
            break;
        }

        let start = char_byte_offsets[char_start];
        let end = if char_end >= char_count { text.len() } else { char_byte_offsets[char_end] };
        let span_text = text[start..end].to_string();
        if span_text.is_empty() {
            continue;
        }

        let style_index = run.style_index as usize;
        let style = if style_index < styles.styles.len() {
            &styles.styles[style_index]
        } else {
            &styles.styles[0]
        };

        // Trust the per-style font_size verbatim. We now source it from
        // Paige's `dword84` (pg_fixed `point`, the real point size) rather
        // than `word46` (RTF stylesheet number), so the historical
        // "word48 == 0 → fall back to document default" override is no
        // longer needed and was actively wrong for the Junkbot help screen
        // (S7.7: dword84 → 6 px body text was being inflated to 12 px
        // because its word48 happens to be 0). The CS Junkbot credits
        // 04b_08 * concern that motivated the override does not regress
        // because dword84 already carries the correct point size for the
        // default style too.
        let html_style = xmed_style_to_html_style(style);
        let color_hex = html_style.color.map(|c| format!("#{:06X}", c & 0xFFFFFF)).unwrap_or_else(|| "none".to_string());
        let bg_hex = html_style.bg_color.map(|c| format!("#{:06X}", c & 0xFFFFFF)).unwrap_or_else(|| "none".to_string());
        debug!(
            "  StyledSpan[{}]: chars[{}..{}]='{}' style_index={} color={} bg={} font='{}' size={:?} bold={} italic={}",
            i, start, end, &span_text[..span_text.len().min(40)],
            style_index, color_hex, bg_hex,
            html_style.font_face.as_deref().unwrap_or("?"),
            html_style.font_size,
            html_style.bold, html_style.italic,
        );

        spans.push(StyledSpan {
            text: span_text,
            style: html_style,
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
    let font_name_lc = xmed_style.font_name.to_ascii_lowercase();
    let mapped_size = if font_name_lc.contains("tiki magic") {
        // This face overflows quickly when upscaled; keep its authored size.
        xmed_style.font_size as i32
    } else if font_name_lc.contains("tiki island") {
        // PFR bitmap font needs more aggressive scaling (~1.4x) to match Director.
        ((xmed_style.font_size as i32 * 7) / 5).max(1)
    } else {
        map_xmed_font_size(xmed_style.font_size as i32)
    };
    debug!(
        "[xmed_style_to_html] font='{}' raw_size={} mapped_size={}",
        xmed_style.font_name, xmed_style.font_size, mapped_size
    );
    HtmlStyle {
        font_face: Some(xmed_style.font_name.clone()),
        font_size: Some(mapped_size),
        color: xmed_style.color.or(Some(xmed_style.fore_color)),
        bg_color: Some(xmed_style.back_color),
        bold: xmed_style.bold,
        italic: xmed_style.italic,
        underline: xmed_style.underline,
        kerning: xmed_style.kerning,
        char_spacing: xmed_style.char_spacing,
    }
}

/// Convert XMED style size units to runtime text size.
/// Pass through directly — the raw XMED size is the Director point size.
fn map_xmed_font_size(raw_size: i32) -> i32 {
    raw_size.max(0)
}
