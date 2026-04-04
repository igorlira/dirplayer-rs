use binary_reader::{BinaryReader, Endian};
use log::{debug, warn};

// Import the new PFR1 parser
use super::pfr1;
use super::pfr1::types::Pfr1ParsedFont;

// Import the XMED styled text parser
pub use super::xmedia_styled_text::XmedStyledText;

pub struct XMediaChunk {
    pub raw_data: Vec<u8>,
}

pub struct PfrFont {
    pub font_name: String,
    pub parsed: Pfr1ParsedFont,
    pub raw_data: Vec<u8>,
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

        debug!("XMED raw_data ({} bytes)", raw_data.len());

        Ok(XMediaChunk { raw_data })
    }

    pub fn is_pfr_font(&self) -> bool {
        if self.raw_data.len() < 100 {
            return false;
        }

        // Check for "PFR1" magic (0x50 0x46 0x52 0x31)
        if self.raw_data.len() >= 4 && &self.raw_data[0..4] == b"PFR1" {
            debug!("Found PFR1 magic header");
            return true;
        }

        false
    }

    pub fn is_styled_text(&self) -> bool {
        if self.raw_data.len() < 12 {
            debug!("XMED data too small ({} bytes)", self.raw_data.len());
            return false;
        }

        // Check for "FFFF" magic (styled text XMED format)
        if &self.raw_data[0..4] == b"FFFF" {
            debug!("Found FFFF styled text header");
            return true;
        }

        debug!(
            "Not FFFF header: {:02X} {:02X} {:02X} {:02X}",
            self.raw_data[0], self.raw_data[1], self.raw_data[2], self.raw_data[3]
        );
        false
    }

    pub fn parse_styled_text(&self) -> Option<XmedStyledText> {
        if !self.is_styled_text() {
            return None;
        }

        debug!("Parsing XMED styled text format...");

        match super::xmedia_styled_text::parse_xmed(&self.raw_data) {
            Ok(styled_text) => {
                debug!("  Text: {} chars", styled_text.text.len());
                debug!("  Spans: {}", styled_text.styled_spans.len());
                debug!("  Alignment: {:?}", styled_text.alignment);

                debug!(
                    "XMED parsed: text='{}' ({} chars), spans={}, alignment={:?}",
                    styled_text.text, styled_text.text.len(), styled_text.styled_spans.len(), styled_text.alignment
                );

                // Log each styled span
                for (idx, span) in styled_text.styled_spans.iter().enumerate() {
                    debug!(
                        "  Span {}: text='{}', font={:?}, size={:?}, bold={}, italic={}, underline={}",
                        idx, span.text,
                        span.style.font_face, span.style.font_size,
                        span.style.bold, span.style.italic, span.style.underline
                    );
                }

                Some(styled_text)
            }
            Err(e) => {
                warn!("Failed to parse XMED styled text: {}", e);
                None
            }
        }
    }

    /// Parse PFR font using the new PFR1 parser
    pub fn parse_pfr_font(&self) -> Option<PfrFont> {
        if !self.is_pfr_font() {
            return None;
        }

        debug!("Parsing PFR font with PFR1 parser ({} bytes)...", self.raw_data.len());

        match pfr1::parse_pfr1_font(&self.raw_data) {
            Ok(parsed) => {
                let font_name = parsed.font_name.clone();
                debug!("PFR1 font parsed: name='{}', {} outline glyphs, {} bitmap glyphs",
                    font_name, parsed.glyphs.len(), parsed.bitmap_glyphs.len());

                Some(PfrFont {
                    font_name,
                    parsed,
                    raw_data: self.raw_data.clone(),
                })
            }
            Err(e) => {
                warn!("PFR1 parsing failed: {}", e);
                None
            }
        }
    }
}
