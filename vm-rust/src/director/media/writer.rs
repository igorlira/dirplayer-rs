use binary_rw::{BinaryWriter, BinaryError, Endian};
use num::ToPrimitive;

use crate::{director::{enums::BitmapInfo, utils::FOURCC}, player::{bitmap::bitmap::{PaletteRef, compress_bitmap, encode_bitmap_data}, cast_member::Media}};

fn map_err(e: BinaryError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::Other, format!("{:?}", e))
}

fn compute_pitch(width: u16, bit_depth: u8) -> u16 {
    // ceil(width * bit_depth / 8) rounded up to the next even number
    let row_bytes = ((width as u32 * bit_depth as u32) + 7) / 8;
    if row_bytes % 2 != 0 { (row_bytes + 1) as u16 } else { row_bytes as u16 }
}

pub trait MediaWriter {
    fn write_media(&mut self, media: &Media, chunk_endian: Endian) -> Result<usize, std::io::Error>;
    fn write_bitmap_media_metadata(&mut self, info: &BitmapInfo) -> Result<usize, std::io::Error>;
}

impl MediaWriter for BinaryWriter<'_> {
    fn write_media(&mut self, media: &Media, chunk_endian: Endian) -> Result<usize, std::io::Error> {
        match media {
            Media::Bitmap { bitmap, reg_point } => {
                let target_bit_depth = bitmap.original_bit_depth;
                let pitch = compute_pitch(bitmap.width, target_bit_depth);

                let info = BitmapInfo {
                    width: bitmap.width,
                    height: bitmap.height,
                    bit_depth: target_bit_depth,
                    pitch,
                    reg_x: reg_point.0,
                    reg_y: reg_point.1,
                    palette_id: match bitmap.palette_ref {
                        PaletteRef::BuiltIn(palette) => palette.to_i16().unwrap_or(-1),
                        _ => -1,
                    },
                    clut_cast_lib: -1,
                    use_alpha: bitmap.use_alpha,
                    trim_white_space: bitmap.trim_white_space,
                    center_reg_point: false,
                };

                // Encode bitmap data to raw bytes
                let raw_data = encode_bitmap_data(bitmap, target_bit_depth, pitch)
                    .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;

                // Compress with PackBits RLE, ONE ROW AT A TIME. Director's
                // BITD/DTIB RLE is per-row: every row of `pitch` bytes is
                // compressed independently and must decode back to exactly
                // `pitch` bytes before the next row begins. Compressing the
                // whole image as a single stream lets a run or literal span a
                // row boundary (e.g. the row's pad byte + the next row's first
                // pixel as one literal). dirplayer's own stream decoder
                // tolerates that, but the real Director Multiuser xtra decodes
                // per row, so a boundary-crossing op makes a row decode to the
                // wrong length, desyncs the bitmap, and drops the connection
                // when a real Shockwave client *views* the photo. Matching the
                // per-row scheme keeps each row aligned to `pitch`.
                let pitch_bytes = pitch as usize;
                let mut compressed = Vec::new();
                if pitch_bytes > 0 {
                    for row in raw_data.chunks(pitch_bytes) {
                        compressed.extend_from_slice(&compress_bitmap(row));
                    }
                } else {
                    compressed = compress_bitmap(&raw_data);
                }

                // Use compressed only if it's actually smaller
                let chunk_data = if compressed.len() < raw_data.len() {
                    compressed
                } else {
                    raw_data
                };

                // The MUS media header encodes the imaging model. Director
                // serializes an indexed (<=8-bit) PixMap with magic
                // 0x18438963, and a direct-color (>=16-bit) image with
                // 0xE8921468. Habbo v7's camera photos are 8-bit #grayscale,
                // so they MUST use the paletted magic — otherwise the real
                // Multiuser xtra can't instantiate the value, the message
                // content comes through VOID, and the client disconnects.
                // The magic value (and the rest of the header) is verified
                // byte-for-byte against the bytes a real Shockwave v7 client
                // sends to the woodpecker server (server log: head=[18 43 89
                // 63 02 00 .. 80 A2 .. C0 08 .. DTIB]). The >8-bit path keeps
                // the pre-existing magic (unverified, left unchanged so
                // truecolor media doesn't regress).
                let magic = if target_bit_depth <= 8 {
                    0x18438963u32
                } else {
                    0xE8921468u32
                };
                let mut written = self.write_u32(magic).map_err(map_err)?;

                // Write metadata
                written += self.write_bitmap_media_metadata(&info)?;

                // Write chunk ID: BITD for big-endian, DTIB for little-endian
                let chunk_id = match chunk_endian {
                    Endian::Big => FOURCC("BITD"),
                    Endian::Little => FOURCC("DTIB"),
                };
                written += self.write_u32(chunk_id).map_err(map_err)?;

                // Write chunk size in the chunk's endianness
                let size_bytes = match chunk_endian {
                    Endian::Big => (chunk_data.len() as u32).to_be_bytes(),
                    Endian::Little => (chunk_data.len() as u32).to_le_bytes(),
                };
                written += self.write_bytes(&size_bytes).map_err(map_err)?;

                written += self.write_bytes(&chunk_data).map_err(map_err)?;
                if chunk_data.len() % 2 != 0 {
                    written += self.write_u8(0).map_err(map_err)?; // Padding byte for even length
                }

                Ok(written)
            }
            _ => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Unsupported media type for writing",
            )),
        }
    }

    fn write_bitmap_media_metadata(&mut self, info: &BitmapInfo) -> Result<usize, std::io::Error> {
        // Indexed bitmaps (2..=8-bit) are QuickDraw PixMaps: rowBytes carries
        // the 0x8000 high-bit flag and the format byte (_unknown_6) is 0xC0.
        // Direct-color / 1-bit images use the plain pitch and 0xA0. The 8-bit
        // case is verified against the real Shockwave v7 photo; the others are
        // left at the previous values to avoid regressing untested paths.
        let is_indexed_pixmap = info.bit_depth >= 2 && info.bit_depth <= 8;
        let row_bytes_field = if is_indexed_pixmap { info.pitch | 0x8000 } else { info.pitch };
        let format_byte: u8 = if is_indexed_pixmap { 0xC0 } else { 0xA0 };

        let mut written = self.write_u16(0x0200u16).map_err(map_err)?;       // _unknown_0: Constant 02 00
        written += self.write_bytes(&[0u8; 14]).map_err(map_err)?;            // _unknown_1: All zeros
        written += self.write_u16(0x0100u16).map_err(map_err)?;               // _unknown_2: Constant 01 00
        written += self.write_bytes(&[0u8; 6]).map_err(map_err)?;             // _unknown_3: All zeros
        written += self.write_u16(row_bytes_field).map_err(map_err)?;        // row_bytes (pitch) [+0x8000 PixMap flag]
        written += self.write_i16(0i16).map_err(map_err)?;                    // rect_top
        written += self.write_i16(0i16).map_err(map_err)?;                    // rect_left
        written += self.write_i16(info.height as i16).map_err(map_err)?;      // rect_bottom
        written += self.write_i16(info.width as i16).map_err(map_err)?;       // rect_right
        written += self.write_bytes(&[0x01]).map_err(map_err)?;               // _unknown_4: Constant 0x01
        written += self.write_bytes(&[0u8; 7]).map_err(map_err)?;             // _unknown_5: All zeros
        written += self.write_i16(info.reg_y).map_err(map_err)?;              // reg_y
        written += self.write_i16(info.reg_x).map_err(map_err)?;              // reg_x
        written += self.write_i8(format_byte as i8).map_err(map_err)?;       // _unknown_6: 0xC0 indexed PixMap / 0xA0 otherwise
        written += self.write_u8(info.bit_depth).map_err(map_err)?;           // depth
        written += self.write_i16(-1i16).map_err(map_err)?;                   // clut_cast_lib: -1 for builtin palettes
        written += self.write_i16(info.palette_id + 1).map_err(map_err)?;     // palette stored as BuiltInPalette enum value + 1
        written += self.write_bytes(&[0x01, 0x00, 0x00, 0x00]).map_err(map_err)?; // _unknown_9: 01 00 00 00
        Ok(written)
    }
}
