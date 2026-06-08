//! Director text encoding.
//!
//! Modern Director files (Windows-authored .dir / .dcr / .cct) store text as
//! Windows-1252 (CP1252). The 0x00-0x7F range is plain ASCII; 0x80-0xFF
//! covers Western European letters including the German umlauts (ä ö ü ß),
//! Scandinavian (å ø æ Å Ø Æ), and Spanish accents (á é í ó ú ñ ¿ ¡).
//!
//! Mac-authored Director files use Mac Roman in the same byte range, but
//! that's vanishingly rare in current test corpora — every reported
//! "missing umlaut" comes from a Windows movie. We default to Win-1252.
//!
//! Five codepoints in 0x80-0x9F are undefined in Win-1252 (0x81, 0x8D,
//! 0x8F, 0x90, 0x9D). We map those to U+FFFD (replacement char) so they
//! surface visibly rather than silently becoming the wrong glyph.
//!
//! References:
//! - https://en.wikipedia.org/wiki/Windows-1252
//! - https://www.unicode.org/Public/MAPPINGS/VENDORS/MICSFT/WINDOWS/CP1252.TXT

/// Windows-1252 byte → Unicode codepoint table for bytes 0x80-0xFF.
/// U+FFFD marks the five officially-undefined positions.
const WIN1252_HIGH: [char; 128] = [
    '\u{20AC}', '\u{FFFD}', '\u{201A}', '\u{0192}', '\u{201E}', '\u{2026}', '\u{2020}', '\u{2021}', // 80-87
    '\u{02C6}', '\u{2030}', '\u{0160}', '\u{2039}', '\u{0152}', '\u{FFFD}', '\u{017D}', '\u{FFFD}', // 88-8F
    '\u{FFFD}', '\u{2018}', '\u{2019}', '\u{201C}', '\u{201D}', '\u{2022}', '\u{2013}', '\u{2014}', // 90-97
    '\u{02DC}', '\u{2122}', '\u{0161}', '\u{203A}', '\u{0153}', '\u{FFFD}', '\u{017E}', '\u{0178}', // 98-9F
    '\u{00A0}', '\u{00A1}', '\u{00A2}', '\u{00A3}', '\u{00A4}', '\u{00A5}', '\u{00A6}', '\u{00A7}', // A0-A7
    '\u{00A8}', '\u{00A9}', '\u{00AA}', '\u{00AB}', '\u{00AC}', '\u{00AD}', '\u{00AE}', '\u{00AF}', // A8-AF
    '\u{00B0}', '\u{00B1}', '\u{00B2}', '\u{00B3}', '\u{00B4}', '\u{00B5}', '\u{00B6}', '\u{00B7}', // B0-B7
    '\u{00B8}', '\u{00B9}', '\u{00BA}', '\u{00BB}', '\u{00BC}', '\u{00BD}', '\u{00BE}', '\u{00BF}', // B8-BF
    '\u{00C0}', '\u{00C1}', '\u{00C2}', '\u{00C3}', '\u{00C4}', '\u{00C5}', '\u{00C6}', '\u{00C7}', // C0-C7
    '\u{00C8}', '\u{00C9}', '\u{00CA}', '\u{00CB}', '\u{00CC}', '\u{00CD}', '\u{00CE}', '\u{00CF}', // C8-CF
    '\u{00D0}', '\u{00D1}', '\u{00D2}', '\u{00D3}', '\u{00D4}', '\u{00D5}', '\u{00D6}', '\u{00D7}', // D0-D7
    '\u{00D8}', '\u{00D9}', '\u{00DA}', '\u{00DB}', '\u{00DC}', '\u{00DD}', '\u{00DE}', '\u{00DF}', // D8-DF
    '\u{00E0}', '\u{00E1}', '\u{00E2}', '\u{00E3}', '\u{00E4}', '\u{00E5}', '\u{00E6}', '\u{00E7}', // E0-E7
    '\u{00E8}', '\u{00E9}', '\u{00EA}', '\u{00EB}', '\u{00EC}', '\u{00ED}', '\u{00EE}', '\u{00EF}', // E8-EF
    '\u{00F0}', '\u{00F1}', '\u{00F2}', '\u{00F3}', '\u{00F4}', '\u{00F5}', '\u{00F6}', '\u{00F7}', // F0-F7
    '\u{00F8}', '\u{00F9}', '\u{00FA}', '\u{00FB}', '\u{00FC}', '\u{00FD}', '\u{00FE}', '\u{00FF}', // F8-FF
];

/// Decode a single Windows-1252 byte into its Unicode `char`.
#[inline]
pub fn win1252_byte_to_char(byte: u8) -> char {
    if byte < 0x80 {
        byte as char
    } else {
        WIN1252_HIGH[(byte - 0x80) as usize]
    }
}

/// Encode a Unicode `char` to its Windows-1252 byte equivalent, returning
/// `None` if the character has no CP1252 mapping. Used by the bitmap-font
/// renderer to look up glyph slots in the PFR1 atlas — a plain `c as u8`
/// truncates higher codepoints to the wrong byte (€ U+20AC -> 0xAC = ¬,
/// ‘ U+2018 -> 0x18 control, em-dash U+2014 -> 0x14 control, etc.) so the
/// user sees random glyphs.
///
/// ASCII (0x00-0x7F) passes through. For 0x80-0xFF, the table is the
/// inverse of `WIN1252_HIGH`. The 5 unassigned positions (0x81, 0x8D,
/// 0x8F, 0x90, 0x9D) have no Unicode codepoint, so they don't appear here.
#[inline]
pub fn char_to_win1252_byte(c: char) -> Option<u8> {
    let cp = c as u32;
    if cp < 0x80 {
        return Some(cp as u8);
    }
    if cp >= 0x00A0 && cp <= 0x00FF {
        // Latin-1 range -- direct mapping for everything except the
        // five points that Win-1252 reassigns in 0x80-0x9F.
        return Some(cp as u8);
    }
    // High-byte non-Latin-1 codepoints: search the WIN1252_HIGH table.
    // Only 27 entries, linear scan is fine and runs once per rendered
    // glyph; this avoids embedding a second 256-entry sparse table.
    for (i, &mapped) in WIN1252_HIGH.iter().enumerate() {
        if (mapped as u32) == cp && mapped != '\u{FFFD}' {
            return Some(0x80 + i as u8);
        }
    }
    None
}

/// Shorthand for the bitmap-font glyph lookup: return the Win-1252 byte
/// for `c`, or `0` (NUL — no advance, no glyph) for chars with no CP1252
/// mapping. Designed to replace the unsafe `c as u8` pattern that
/// silently truncated codepoints like € (U+20AC) into the wrong glyph cell.
#[inline]
pub fn glyph_byte_for(c: char) -> u8 {
    char_to_win1252_byte(c).unwrap_or(0)
}

/// Decode a slice of Windows-1252 bytes into a Rust `String`.
/// Replaces the old `String::from_utf8_lossy(bytes).into_owned()` pattern,
/// which mangled every non-ASCII byte into U+FFFD because Director text is
/// never UTF-8.
pub fn decode_win1252(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        s.push(win1252_byte_to_char(b));
    }
    s
}

/// Decode bytes that came from an external source (HTTP response body,
/// local FileIO read, XML payload) where the encoding isn't recorded
/// in-band. Strategy:
///
/// 1. If the bytes start with a UTF-8 BOM (`EF BB BF`), strip it.
/// 2. Try strict UTF-8. If valid, use it.
/// 3. Otherwise fall back to Windows-1252.
///
/// This is unambiguous in practice because valid UTF-8 multi-byte sequences
/// almost never form by accident when the source is Win-1252: a high byte
/// (≥ 0x80) in CP1252 is a standalone character, while UTF-8 requires high
/// bytes to come in well-formed continuation patterns. The check is cheap
/// (`std::str::from_utf8` is O(n) and bails on the first invalid sequence),
/// so the fallback only runs when the strict path actually fails.
pub fn decode_text_auto(bytes: &[u8]) -> String {
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => decode_win1252(bytes),
    }
}

/// Mac Roman byte → Unicode codepoint table for bytes 0x80-0xFF.
///
/// Director historically stored Lingo SCRIPT string literals (the Lscr
/// literal store) in Mac Roman regardless of the authoring/packaging
/// platform — Director originated on the Mac and kept Mac Roman as the
/// canonical script-text encoding. So a Windows-packaged (XFIR) movie can
/// still carry Mac Roman bytes in its script literals: e.g. the section
/// sign `§` is byte 0xA4 in Mac Roman (it is `¤` in Win-1252). The
/// reference decompiler (ProjectorRays) likewise decodes Lscr strings as
/// Mac Roman. Member/field text is a separate concern (UTF-8 or Win-1252)
/// and is NOT decoded with this table.
///
/// Values follow Apple's canonical `ROMAN.TXT` mapping (post-euro: 0xDB =
/// €). 0xF0 is the Apple-logo PUA codepoint U+F8FF.
const MACROMAN_HIGH: [char; 128] = [
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

/// Decode a single Mac Roman byte into its Unicode `char`.
#[inline]
pub fn macroman_byte_to_char(byte: u8) -> char {
    if byte < 0x80 {
        byte as char
    } else {
        MACROMAN_HIGH[(byte - 0x80) as usize]
    }
}

/// Decode a slice of Mac Roman bytes into a Rust `String`.
pub fn decode_macroman(bytes: &[u8]) -> String {
    let mut s = String::with_capacity(bytes.len());
    for &b in bytes {
        s.push(macroman_byte_to_char(b));
    }
    s
}

/// Like [`decode_text_auto`] but falls back to **Mac Roman** instead of
/// Windows-1252 when the bytes aren't valid UTF-8. Use this for Lingo
/// script string literals, which Director stores in Mac Roman on every
/// platform (see [`MACROMAN_HIGH`]). Pure-ASCII and genuine UTF-8 (D11+
/// Unicode authoring) still decode correctly via the strict-UTF-8 first
/// pass; only the single-byte fallback differs.
pub fn decode_text_auto_macroman(bytes: &[u8]) -> String {
    let bytes = if bytes.starts_with(&[0xEF, 0xBB, 0xBF]) {
        &bytes[3..]
    } else {
        bytes
    };
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_owned(),
        Err(_) => decode_macroman(bytes),
    }
}
