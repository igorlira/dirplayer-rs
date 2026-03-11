use binary_reader::BinaryReader;
use anyhow::{bail, Result};

use crate::io::reader::DirectorExt;

pub struct TextChunk {
    pub offset: usize,
    pub text_length: usize,
    pub data_length: usize,
    pub text: String,
    pub data: Vec<u8>,
}

/// Parsed formatting run from STXT data section
#[derive(Debug, Clone)]
pub struct StxtFormattingRun {
    pub start_position: u32,
    pub height: u16,
    pub ascent: u16,
    pub font_id: u16,
    pub style: u8,
    pub font_size: u16,
    pub color_r: u16,  // QuickDraw 16-bit Red
    pub color_g: u16,  // QuickDraw 16-bit Green
    pub color_b: u16,  // QuickDraw 16-bit Blue
}

impl TextChunk {
    pub fn read(reader: &mut BinaryReader) -> Result<TextChunk> {
        reader.set_endian(binary_reader::Endian::Big);

        let offset = reader.read_usize32()?;
        if offset != 12 {
            bail!("Stxt init: unhandled offset");
        }

        let text_length = reader.read_usize32()?;
        let data_length = reader.read_usize32()?;

        Ok(TextChunk {
            offset,
            text_length,
            data_length,
            text: reader.read_string(text_length)?,
            data: reader.read_bytes(data_length)?.to_vec(),
        })
    }

    /// Parse formatting runs from the STXT data section.
    /// Each run is 20 bytes: startPos(4) + height(2) + ascent(2) + fontId(2) + style(1) + reserved(1) + fontSize(2) + colorR(2) + colorG(2) + colorB(2)
    pub fn parse_formatting_runs(&self) -> Vec<StxtFormattingRun> {
        let data = &self.data;
        if data.len() < 2 {
            return Vec::new();
        }

        let num_runs = ((data[0] as u16) << 8) | (data[1] as u16);
        let mut runs = Vec::new();

        for i in 0..num_runs as usize {
            let offset = 2 + i * 20;
            if offset + 20 > data.len() {
                break;
            }

            let start_position = ((data[offset] as u32) << 24)
                | ((data[offset + 1] as u32) << 16)
                | ((data[offset + 2] as u32) << 8)
                | (data[offset + 3] as u32);
            let height = ((data[offset + 4] as u16) << 8) | (data[offset + 5] as u16);
            let ascent = ((data[offset + 6] as u16) << 8) | (data[offset + 7] as u16);
            let font_id = ((data[offset + 8] as u16) << 8) | (data[offset + 9] as u16);
            let style = data[offset + 10];
            // data[offset + 11] is reserved
            let font_size = ((data[offset + 12] as u16) << 8) | (data[offset + 13] as u16);
            let color_r = ((data[offset + 14] as u16) << 8) | (data[offset + 15] as u16);
            let color_g = ((data[offset + 16] as u16) << 8) | (data[offset + 17] as u16);
            let color_b = ((data[offset + 18] as u16) << 8) | (data[offset + 19] as u16);

            runs.push(StxtFormattingRun {
                start_position,
                height,
                ascent,
                font_id,
                style,
                font_size,
                color_r,
                color_g,
                color_b,
            });
        }

        runs
    }
}
