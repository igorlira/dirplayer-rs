use binary_reader::{BinaryReader, Endian};

use crate::{director::{enums::BitmapInfo, utils::FOURCC}, player::{bitmap::bitmap::decompress_bitmap, cast_member::{Media}}};

pub trait MediaReader {
    fn read_media(&mut self) -> Result<Media, std::io::Error>;
    fn read_bitmap_media_metadata(&mut self) -> Result<BitmapInfo, std::io::Error>;
}

impl MediaReader for BinaryReader {
    fn read_media(&mut self) -> Result<Media, std::io::Error> {
        let magic = self.read_u32()?; // Bitmap: e8 92 14 68
        match magic {
            0xE8921468 => {
                let metadata = self.read_bitmap_media_metadata()?;
                let chunk_id = self.read_u32()?;
                let endian = if chunk_id == FOURCC("BITD") {
                    Endian::Big
                } else if chunk_id == FOURCC("DTIB") {
                    Endian::Little
                } else {
                    return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Unknown media chunk ID: {}", chunk_id)));
                };
                self.set_endian(endian);
                let chunk_size = self.read_u32()? as usize;
                let chunk_data = self.read_bytes(chunk_size)?.to_vec();
                let decompressed = decompress_bitmap(&chunk_data, &metadata, 0, 0).unwrap();
                Ok(Media::Bitmap { bitmap: decompressed, reg_point: (metadata.reg_y, metadata.reg_x) })
            }
            _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("Unknown media magic number: {}", magic))),
        }
    }

    fn read_bitmap_media_metadata(&mut self) -> Result<BitmapInfo, std::io::Error> {
        let _unknown_0 = self.read_u16()?; // Constant: 02 00
        let _unknown_1 = self.read_bytes(14)?; // All zeros
        let _unknown_2 = self.read_u16()?; // Constant: 01 00
        let _unknown_3 = self.read_bytes(6)?; // All zeros
        let row_bytes = self.read_u16()? & 0x7FFF; // High bit is set for compressed bitmaps, but MUS messages always use uncompressed. Equals ceil(width * bitDepth / 8) rounded up to the next even number.
        let rect_top = self.read_i16()?;
        let rect_left = self.read_i16()?;
        let rect_bottom = self.read_i16()?;
        let rect_right = self.read_i16()?;
        let _unknown_4 = self.read_bytes(1)?; // Constant: 0x01
        let _unknown_5 = self.read_bytes(7)?; // All zeros
        let reg_y = self.read_i16()?;
        let reg_x = self.read_i16()?;
        let _unknown_6 = self.read_i8()?; // Constant: 0xa0
        let depth = self.read_u8()?;
        let clut_cast_lib = self.read_i16()?; // -1 for builtin palettes
        let palette_stored = self.read_i16()?; // BuiltInPalette enum value + 1
        let _unknown_9 = self.read_bytes(4)?; // 01 00 00 00

        return Ok(BitmapInfo {
            pitch: row_bytes,
            width: (rect_right - rect_left) as u16,
            height: (rect_bottom - rect_top) as u16,
            reg_y,
            reg_x,
            bit_depth: depth,
            palette_id: palette_stored - 1,
            clut_cast_lib,
            use_alpha: false,
            trim_white_space: false,
            center_reg_point: false,
        });
    }
}