//! Parser for the `XTRl` chunk — Director's list of xtras a movie
//! depends on (the `theXtraList` Lingo property is built from this).
//!
//! ## Why this exists
//!
//! When dirplayer-rs loads an external WASM xtra plugin (via the
//! `xtra-sdk` system), the host needs to know **which** xtras a movie
//! requires so it can resolve them through the host's name→URL registry
//! and fetch the right .wasm files before Lingo runs. Parsing XTRl
//! turns that into "movie says I need BobbaXtra, host registry maps
//! it to https://…/bobba.wasm".
//!
//! ## Wire format
//!
//! Reverse-engineered from a sample chunk (see `external-xtra-plugin-spec`
//! discussion). Neither ScummVM nor ProjectorRays parses XTRl —
//! ScummVM treats individual `Xtra` chunks as skip-only, and the LIST
//! chunk (`XTRl`) isn't covered.
//!
//! Per-entry structure observed (sizes here are illustrative):
//!
//! ```text
//! u32 BE  entry_size          73
//! u32 BE  sub_header_size    24
//! u32 BE  clsid_len          16
//! [16]    clsid               zeros for scripting xtras, real UUID for asset xtras
//! ...     misc header        4-byte flags / counters
//! Pascal-style strings prefixed by 06 05 (filename) or 06 02 (display):
//!     06 05 LEN <ascii>      e.g. "BobbaXtra.x32"
//!     06 02 LEN <ascii>      e.g. "BobbaXtra"
//! ```
//!
//! The header layout has some endian quirks across entries that aren't
//! worth perfectly reversing for the use case. We instead scan the
//! entire chunk payload for `06 05 LEN <bytes>` markers (each marker is
//! immediately followed by a length byte that fits in a u8 and a span
//! of printable ASCII), and optionally pick up the `06 02 LEN <bytes>`
//! display name that usually follows after some NUL padding. This is
//! resilient to per-entry header weirdness and trivially handles future
//! header layout changes.

use binary_reader::BinaryReader;

/// One xtra dependency declared by the movie.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct XtraDecl {
    /// The plugin filename as the movie declares it, e.g. `"BobbaXtra.x32"`
    /// or `"Shockwave 3D Asset.x32"`. Case is preserved; matching against
    /// the host's registry should be case-insensitive.
    pub filename: String,
    /// Human-readable display name as the movie declares it, e.g.
    /// `"BobbaXtra"`. May be `None` for older / system xtras whose
    /// entry only has the filename.
    pub display_name: Option<String>,
}

/// Parsed XTRl chunk — the movie's xtra dependency list.
#[derive(Debug, Clone, Default)]
pub struct XtraListChunk {
    pub entries: Vec<XtraDecl>,
}

impl XtraListChunk {
    /// Read an XTRl chunk from a `BinaryReader`. Matches the
    /// `from_reader` shape used by other chunks in this module. Cannot
    /// fail meaningfully — malformed entries are skipped, never panic.
    pub fn from_reader(reader: &mut BinaryReader) -> Result<Self, String> {
        let remaining = reader.length - reader.pos;
        let bytes = reader
            .read_bytes(remaining)
            .map_err(|e| format!("XtraListChunk: failed to read payload: {}", e))?
            .to_vec();
        Ok(Self {
            entries: parse_xtra_list(&bytes),
        })
    }
}

/// Parse an XTRl chunk payload. Returns the declared xtras in file order.
/// Never panics; malformed entries are skipped silently.
pub fn parse_xtra_list(payload: &[u8]) -> Vec<XtraDecl> {
    let mut out = Vec::new();
    let mut i = 0;
    while i + 3 < payload.len() {
        // Look for a filename marker: 06 05 LEN <ascii>.
        if payload[i] == 0x06 && payload[i + 1] == 0x05 {
            let len = payload[i + 2] as usize;
            let str_start = i + 3;
            let str_end = str_start + len;
            if str_end <= payload.len() {
                if let Some(filename) = decode_ascii(&payload[str_start..str_end]) {
                    // Optional display name: skip NUL pad, look for 06 02 LEN <ascii>.
                    let display_name = read_display_name_after(payload, str_end);
                    // Advance past whichever was last consumed. We don't
                    // need to be precise — the next entry's 06 05 marker
                    // anchors us again.
                    let advance_to = display_name
                        .as_ref()
                        .map(|d| {
                            let mut j = str_end;
                            while j < payload.len() && payload[j] == 0 {
                                j += 1;
                            }
                            // We know there's a 06 02 LEN at j; advance past it + its string.
                            j + 3 + d.len()
                        })
                        .unwrap_or(str_end);
                    out.push(XtraDecl {
                        filename,
                        display_name,
                    });
                    i = advance_to;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn read_display_name_after(payload: &[u8], filename_end: usize) -> Option<String> {
    let mut j = filename_end;
    while j < payload.len() && payload[j] == 0 {
        j += 1;
    }
    if j + 3 > payload.len() {
        return None;
    }
    if payload[j] != 0x06 || payload[j + 1] != 0x02 {
        return None;
    }
    let len = payload[j + 2] as usize;
    let str_start = j + 3;
    let str_end = str_start + len;
    if str_end > payload.len() {
        return None;
    }
    decode_ascii(&payload[str_start..str_end])
}

/// Returns the decoded String only if every byte is printable ASCII (or
/// space). This keeps us from picking up spurious matches in binary
/// data (CLSIDs, length fields, etc.) that happen to start with 06 05.
fn decode_ascii(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return None;
    }
    for &b in bytes {
        // Printable ASCII range, plus space. Reject control characters,
        // high-bit-set bytes, and embedded NULs.
        if !(0x20..=0x7e).contains(&b) {
            return None;
        }
    }
    Some(String::from_utf8_lossy(bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal hand-crafted sample showing one entry with filename + display.
    #[test]
    fn parses_single_entry() {
        let mut data = Vec::new();
        // Filename: "Foo.x32" (length 7)
        data.extend_from_slice(&[0x06, 0x05, 7, b'F', b'o', b'o', b'.', b'x', b'3', b'2']);
        // Padding zero between
        data.push(0);
        // Display: "Foo" (length 3)
        data.extend_from_slice(&[0x06, 0x02, 3, b'F', b'o', b'o']);

        let decls = parse_xtra_list(&data);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].filename, "Foo.x32");
        assert_eq!(decls[0].display_name.as_deref(), Some("Foo"));
    }

    #[test]
    fn skips_filename_with_garbage_bytes() {
        // 06 05 with a length that points into binary garbage should be
        // rejected by `decode_ascii`, not produce a junk entry.
        let data = [0x06, 0x05, 4, 0x00, 0xff, 0x80, 0x01];
        assert!(parse_xtra_list(&data).is_empty());
    }

    #[test]
    fn handles_filename_without_display() {
        let data = [0x06, 0x05, 5, b'B', b'a', b'r', b'.', b'x'];
        let decls = parse_xtra_list(&data);
        assert_eq!(decls.len(), 1);
        assert_eq!(decls[0].filename, "Bar.x");
        assert_eq!(decls[0].display_name, None);
    }

    #[test]
    fn finds_multiple_entries_with_garbage_between() {
        let mut data = Vec::new();
        data.extend_from_slice(&[0x06, 0x05, 3, b'A', b'.', b'X']);
        // Some inter-entry header bytes
        data.extend_from_slice(&[0, 0, 0, 0x4b, 0, 0, 0, 0x18, 0, 0, 0, 0x10]);
        data.extend_from_slice(&[0xde, 0xad, 0xbe, 0xef]); // junk
        data.extend_from_slice(&[0x06, 0x05, 3, b'B', b'.', b'X']);
        data.push(0);
        data.extend_from_slice(&[0x06, 0x02, 1, b'B']);

        let decls = parse_xtra_list(&data);
        assert_eq!(decls.len(), 2);
        assert_eq!(decls[0].filename, "A.X");
        assert_eq!(decls[0].display_name, None);
        assert_eq!(decls[1].filename, "B.X");
        assert_eq!(decls[1].display_name.as_deref(), Some("B"));
    }
}
