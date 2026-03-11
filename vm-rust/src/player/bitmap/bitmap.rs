use std::{sync::Arc, vec};

use binary_reader::BinaryReader;
use log::warn;
use num::ToPrimitive;
use num_derive::{FromPrimitive, ToPrimitive};
use std::convert::TryInto;

use crate::{
    director::enums::BitmapInfo,
    player::{
        cast_lib::CastMemberRef, handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
        sprite::ColorRef,
    },
};
use num::FromPrimitive;
use crate::player::cast_lib::cast_member_ref;
use super::{
    mask::BitmapMask,
    palette::{
        GRAYSCALE_16_PALETTE, GRAYSCALE_4_PALETTE, GRAYSCALE_PALETTE, MAC_16_PALETTE,
        METALLIC16_PALETTE, METALLIC_PALETTE, NTSC16_PALETTE, NTSC_PALETTE, PASTELS16_PALETTE,
        PASTELS_PALETTE, RAINBOW16_PALETTE, RAINBOW_PALETTE, SYSTEM_MAC_PALETTE,
        SYSTEM_WIN_PALETTE, VIVID16_PALETTE, VIVID_PALETTE, WEB_216_PALETTE, WIN_16_PALETTE,
    },
    palette_map::PaletteMap,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PaletteRef {
    BuiltIn(BuiltInPalette),
    Member(CastMemberRef),
    /// Use the movie's default palette (first available custom palette, or system palette if none)
    /// This is used when palette_id=0 (meaning "use default" rather than a specific member)
    Default,
}

impl PaletteRef {
    /// Create a PaletteRef from parsed palette_id and clut_cast_lib values.
    ///
    /// - i < 0: builtin palette enum value (e.g., -1=SystemMac, -3=GrayScale)
    /// - i > 0: custom palette member number
    /// - clut_cast_lib: the cast lib containing the palette (0 = search all cast libs)
    pub fn from(i: i16, clut_cast_lib: i16, bitmap_cast_lib: u32) -> Self {
        if i < 0 {
            match BuiltInPalette::from_i16(i) {
                Some(palette) => PaletteRef::BuiltIn(palette),
                None => {
                    web_sys::console::warn_1(
                        &format!("Unknown built-in palette ID: {}, defaulting to SystemWin", i).into()
                    );
                    PaletteRef::BuiltIn(BuiltInPalette::SystemWin)
                }
            }
        } else if i == 0 {
            PaletteRef::BuiltIn(get_system_default_palette())
        } else {
            // clut_cast_lib >= 0: use as-is (0 = search all, >0 = explicit cast lib)
            // clut_cast_lib < 0: not set, use bitmap's own cast lib (ScummVM: _cast->_castLibID)
            let cast_lib = if clut_cast_lib >= 0 {
                clut_cast_lib as i32
            } else {
                bitmap_cast_lib as i32
            };
            PaletteRef::Member(CastMemberRef {
                cast_lib,
                cast_member: i as i32,
            })
        }
    }
}

#[derive(Debug, Clone, Copy, ToPrimitive, FromPrimitive, PartialEq, Eq)]
pub enum BuiltInPalette {
    GrayScale = -3,
    Pastels = -4,
    Vivid = -5,
    Ntsc = -6,
    Metallic = -7,
    Web216 = -8,
    Vga = -9,
    SystemWinDir4 = -101,
    SystemWin = -102,
    SystemMac = -1,
    Rainbow = -2,
}

impl BuiltInPalette {
    pub fn from_symbol_string(symbol: &str) -> Option<Self> {
        match symbol {
            "grayscale" => Some(BuiltInPalette::GrayScale),
            "pastels" => Some(BuiltInPalette::Pastels),
            "vivid" => Some(BuiltInPalette::Vivid),
            "ntsc" => Some(BuiltInPalette::Ntsc),
            "metallic" => Some(BuiltInPalette::Metallic),
            "web216" => Some(BuiltInPalette::Web216),
            "vga" => Some(BuiltInPalette::Vga),
            "systemWinDir4" => Some(BuiltInPalette::SystemWinDir4),
            "systemWin" => Some(BuiltInPalette::SystemWin),
            "systemMac" => Some(BuiltInPalette::SystemMac),
            "rainbow" => Some(BuiltInPalette::Rainbow),
            _ => None,
        }
    }

    pub fn symbol_string(&self) -> String {
        match self {
            BuiltInPalette::GrayScale => "grayscale",
            BuiltInPalette::Pastels => "pastels",
            BuiltInPalette::Vivid => "vivid",
            BuiltInPalette::Ntsc => "ntsc",
            BuiltInPalette::Metallic => "metallic",
            BuiltInPalette::Web216 => "web216",
            BuiltInPalette::Vga => "vga",
            BuiltInPalette::SystemWinDir4 => "systemWinDir4",
            BuiltInPalette::SystemWin => "systemWin",
            BuiltInPalette::SystemMac => "systemMac",
            BuiltInPalette::Rainbow => "rainbow",
        }
        .to_string()
    }
}

pub fn get_system_default_palette() -> BuiltInPalette {
    // TODO: Properly detect platform from movie file format
    BuiltInPalette::SystemWin
}

#[derive(Clone)]
pub struct Bitmap {
    pub width: u16,
    pub height: u16,
    pub bit_depth: u8,          // Current storage format
    pub original_bit_depth: u8, // Original format (for palette selection)
    pub data: Vec<u8>,          // RGBA
    pub palette_ref: PaletteRef,
    pub matte: Option<Arc<BitmapMask>>,
    pub use_alpha: bool,
    pub trim_white_space: bool,
    pub was_trimmed: bool,
    /// Version counter for cache invalidation (incremented when bitmap data changes)
    pub version: u32,
}

impl Bitmap {
    pub fn new(
        width: u16,
        height: u16,
        bit_depth: u8,
        original_bit_depth: u8,
        alpha_depth: u8,
        palette_ref: PaletteRef,
    ) -> Self {
        let bytes_per_pixel = bit_depth as usize / 8;
        let initial_color = match bit_depth {
            16 | 32 => 255,
            _ => 0,
        };

        let data = vec![initial_color; width as usize * height as usize * bytes_per_pixel];

        // For 32-bit images, always create a matte OR handle alpha in the data
        let matte = if alpha_depth > 0 || bit_depth == 32 {
            Some(Arc::new(BitmapMask::new(
                width.try_into().unwrap(),
                height.try_into().unwrap(),
                true, // fill mask if alpha exists
            )))
        } else {
            None
        };

        Self {
            width,
            height,
            bit_depth,
            original_bit_depth,
            data,
            palette_ref,
            matte,
            use_alpha: false,
            trim_white_space: false,
            was_trimmed: false,
            version: 0,
        }
    }

    /// Increment the version counter to indicate the bitmap data has changed.
    /// This is used by the WebGL2 texture cache to know when to re-upload textures.
    pub fn mark_dirty(&mut self) {
        self.version = self.version.wrapping_add(1);
    }
}

fn get_num_channels(bit_depth: u8) -> Result<u8, String> {
    match bit_depth {
        1 | 2 | 4 | 8 => Ok(1),  // 8-bit and below: 1 byte per pixel
        16 => Ok(2),              // 16-bit: 2 bytes per pixel
        32 => Ok(4),              // 32-bit: 4 bytes per pixel
        _ => Err("Invalid bit depth".to_string()),
    }
}

fn get_alignment_width(bit_depth: u8) -> Result<u16, String> {
    match bit_depth {
        // 1-bit: rows aligned to 16-bit (word) boundaries = 16 pixels per row minimum
        1 => Ok(16),
        4 | 32 => Ok(4),
        2 | 8 => Ok(2),
        16 => Ok(1),  // 16-bit aligns like 8-bit (1 byte per pixel)
        _ => Err("Invalid bit depth".to_string()),
    }
}

fn decode_bitmap_1bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    // Decodes 1-bit to 8-bit indexed
    let mut scan_data = vec![0; data.len() * 8];
    let mut p = 0;
    for i in 0..data.len() {
        let byte = data[i];
        for j in 1..=8 {
            let bit = (byte & (0x1 << (8 - j))) >> (8 - j);
            scan_data[p] = if bit == 1 { 0xFF } else { 0x00 };
            p += 1;
        }
    }

    let mut result = vec![0; width as usize * height as usize];
    for y in 0..scan_height {
        for x in 0..scan_width {
            // Use usize arithmetic to avoid u16 overflow for large images
            // e.g., y=152, scan_width=432 -> 152*432=65664 which overflows u16 (max 65535)
            let scan_index = y as usize * scan_width as usize + x as usize;
            if x < width {
                let pixel_index = y as usize * width as usize + x as usize;
                if scan_index >= scan_data.len() {
                    return Err(format!(
                        "decode_bitmap_1bit: scan_index {} >= scan_data.len() {}",
                        scan_index,
                        scan_data.len()
                    ));
                }
                let pixel = scan_data[scan_index];
                result[pixel_index] = pixel;
            }
        }
    }

    Ok(Bitmap {
        bit_depth: 8,
        original_bit_depth: 1, // Keep original 1-bit depth for proper Director rendering semantics
        width,
        height,
        data: result,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
    })
}

fn decode_bitmap_2bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    let mut decoded_data = Vec::new();

    for i in 0..data.len() {
        let original_value = data[i];
        let left_value = (original_value & 0xC0) >> 6;
        let middle_left_value = (original_value & 0x30) >> 4;
        let middle_right_value = (original_value & 0x0C) >> 2;
        let right_value = original_value & 0x03;

        // Keep raw 2-bit palette indices (0-3), like 4-bit decode keeps 0-15.
        // Color resolution happens later via palette lookup.
        decoded_data.push(left_value);
        decoded_data.push(middle_left_value);
        decoded_data.push(middle_right_value);
        decoded_data.push(right_value);
    }

    let mut result_bmp = vec![0; width as usize * height as usize];
    for y in 0..scan_height {
        for x in 0..scan_width {
            let compressed_index = y as usize * scan_width as usize + x as usize;
            if compressed_index >= decoded_data.len() {
                return Err(format!(
                    "decode_bitmap_2bit: compressed_index {} >= decoded_data.len() {}",
                    compressed_index,
                    decoded_data.len()
                ));
            }
            if x < width {
                let pixel_index = y as usize * width as usize + x as usize;
                let pixel = decoded_data[compressed_index];
                result_bmp[pixel_index] = pixel;
            }
        }
    }

    Ok(Bitmap {
        bit_depth: 8,
        original_bit_depth: 2,
        width,
        height,
        data: result_bmp,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
    })
}

fn decode_bitmap_4bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    // Decode 4-bit data to 8-bit indexed (each nibble becomes a byte with value 0-15)
    let mut decoded_data = Vec::new();

    for i in 0..data.len() {
        let original_value = data[i];
        let left_value = (original_value & 0xF0) >> 4;
        let right_value = original_value & 0x0F;

        decoded_data.push(left_value);
        decoded_data.push(right_value);
    }

    // Create result as 8-bit indexed (one byte per pixel)
    let mut result_bmp = vec![0; width as usize * height as usize];

    for y in 0..height {
        for x in 0..width {
            let scan_index = y as usize * scan_width as usize + x as usize;

            if scan_index >= decoded_data.len() {
                return Err(format!(
                    "decode_bitmap_4bit: scan_index {} >= decoded_data.len() {}",
                    scan_index,
                    decoded_data.len()
                ));
            }

            let pixel = decoded_data[scan_index];
            let pixel_index = y as usize * width as usize + x as usize;

            // Store as 8-bit indexed (values 0-15)
            result_bmp[pixel_index] = pixel;
        }
    }

    Ok(Bitmap {
        bit_depth: 8,          // Stored as 8-bit
        original_bit_depth: 4, // But was originally 4-bit
        width,
        height,
        data: result_bmp,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
    })
}

fn decode_bitmap_16bit(
    width: u16,
    height: u16,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
    skip_compression: bool,
) -> Result<Bitmap, String> {
    let expected_size = scan_width as usize * scan_height as usize * 2;

    if data.len() < expected_size {
        return Err(format!(
            "16-bit bitmap: insufficient data (got {}, expected {})",
            data.len(), expected_size
        ));
    }

    let mut result = vec![0u8; width as usize * height as usize * 4];

    for y in 0..height as usize {
        for x in 0..width as usize {
            let pixel16: u16 = if skip_compression {
                // Uncompressed: sequential bytes, 2 per pixel
                // High byte followed by low byte
                let offset = (y * scan_width as usize + x) * 2;
                let high = data[offset];
                let low = data[offset + 1];
                u16::from_be_bytes([high, low])
            } else {
                // Compressed (RLE-decoded): planar per scanline
                // For each row: all high bytes, then all low bytes
                let row_offset = y * scan_width as usize * 2;
                let high = data[row_offset + x];
                let low = data[row_offset + scan_width as usize + x];
                u16::from_be_bytes([high, low])
            };

            // RGB555 - extract 5-bit components
            let r5 = ((pixel16 >> 10) & 0x1F) as u8;
            let g5 = ((pixel16 >> 5) & 0x1F) as u8;
            let b5 = (pixel16 & 0x1F) as u8;

            // Convert 5-bit to 8-bit by shifting left and filling lower bits
            let dst = (y * width as usize + x) * 4;
            result[dst]     = (r5 << 3) | (r5 >> 2);
            result[dst + 1] = (g5 << 3) | (g5 >> 2);
            result[dst + 2] = (b5 << 3) | (b5 >> 2);
            result[dst + 3] = 255;
        }
    }

    Ok(Bitmap {
        width,
        height,
        bit_depth: 32,
        original_bit_depth: 16,
        data: result,
        palette_ref,
        matte: None,
        use_alpha: false,
        trim_white_space: false,
        was_trimmed: false,
        version: 0,
    })
}

fn decode_generic_bitmap(
    width: u16,
    height: u16,
    bit_depth: u8,
    num_channels: u8,
    scan_width: u16,
    scan_height: u16,
    palette_ref: PaletteRef,
    data: &[u8],
) -> Result<Bitmap, String> {
    // Sanity check: prevent capacity overflow from garbage BitmapInfo values
    const MAX_BITMAP_PIXELS: usize = 8192 * 8192; // 64 megapixels
    let total_pixels = width as usize * height as usize;
    if total_pixels > MAX_BITMAP_PIXELS {
        return Err(format!(
            "decode_generic_bitmap: bitmap {}x{} exceeds maximum size",
            width, height
        ));
    }

    let bytes_per_pixel = bit_depth / 8;
    let expected_size = scan_width as usize * scan_height as usize * num_channels as usize * bytes_per_pixel as usize;

    if expected_size != data.len() {
        warn!(
            "decode_generic_bitmap: Expected {} bytes, got {}",
            expected_size,
            data.len()
        );
        let actual_bit_depth = bit_depth * num_channels;
        return Ok(Bitmap::new(
            width,
            height,
            actual_bit_depth,
            bit_depth,
            0,
            palette_ref,
        ));
    } else {
        let mut result =
            vec![
                0;
                width as usize * height as usize * num_channels as usize * bytes_per_pixel as usize
            ];

        // FIX: The indexing was wrong - channels and bytes should be multiplied, not added
        for y in 0..scan_height {
            for x in 0..scan_width {
                if x >= width {
                    continue;
                }
                for c in 0..num_channels {
                    for b in 0..bytes_per_pixel {
                        let scan_index = (y as usize
                            * scan_width as usize
                            * num_channels as usize
                            * bytes_per_pixel as usize)
                            + (x as usize * num_channels as usize * bytes_per_pixel as usize)
                            + (c as usize * bytes_per_pixel as usize)
                            + b as usize;

                        let result_index = (y as usize
                            * width as usize
                            * num_channels as usize
                            * bytes_per_pixel as usize)
                            + (x as usize * num_channels as usize * bytes_per_pixel as usize)
                            + (c as usize * bytes_per_pixel as usize)
                            + b as usize;

                        if scan_index >= data.len() || result_index >= result.len() {
                            warn!(
                                "decode_generic_bitmap: scan_index {} >= data.len() {} or result_index {} >= result.len() {}",
                                scan_index,
                                data.len(),
                                result_index,
                                result.len()
                            );
                            continue;
                        }
                        result[result_index] = data[scan_index];
                    }
                }
            }
        }

        let actual_bit_depth = bit_depth * num_channels;
        return Ok(Bitmap {
            width,
            height,
            bit_depth: actual_bit_depth,
            original_bit_depth: bit_depth,
            data: result,
            palette_ref,
            matte: None,
            use_alpha: false,
            trim_white_space: false,
            was_trimmed: false,
            version: 0,
        });
    }
}

pub fn bitmap_to_hex_string(bitmap: &Bitmap) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "# width={} height={} bit_depth={} data_len={}\n",
        bitmap.width,
        bitmap.height,
        bitmap.bit_depth,
        bitmap.data.len()
    ));

    let bytes_per_row = (bitmap.data.len() as f64 / bitmap.height as f64).ceil() as usize;
    for (i, b) in bitmap.data.iter().enumerate() {
        s.push_str(&format!("{:02X}", b));
        if (i + 1) % bytes_per_row == 0 {
            s.push('\n');
        } else {
            s.push(' ');
        }
    }

    s
}

// Converts a NUU-encoded bitmap to a raw bitmap
pub fn decompress_bitmap(
    data: &[u8],
    info: &BitmapInfo,
    cast_lib: u32,
    version: u16,
) -> Result<Bitmap, String> {
    // Check if the BITD data is actually JPEG-compressed
    if data.len() >= 4 && data[0] == 0xFF && data[1] == 0xD8 && data[2] == 0xFF {
        return decode_jpeg_bitd(data, info, cast_lib);
    }

    // Use clutCastLib from bitmap data if explicitly specified (> 0).
    // clut_cast_lib == 0 means "search all castLibs" — use 0 as sentinel so
    // resolve_unresolved_palette_refs can find the correct palette.
    // clut_cast_lib == -1 means "not set" — use the bitmap's own castLib.
    let palette_cast_lib = if info.clut_cast_lib > 0 {
        info.clut_cast_lib as u32
    } else if info.clut_cast_lib == 0 && info.palette_id > 0 {
        0 // sentinel: search all castLibs during resolution
    } else {
        cast_lib
    };

    let mut result = Vec::new();
    let mut _current_index = 0;
    let num_channels = get_num_channels(info.bit_depth)?;
    let alignment_width = get_alignment_width(info.bit_depth)?;

    let mut reader = BinaryReader::from_u8(data);

    let scan_height = info.height;
    let mut scan_width = if info.pitch > 0 && info.bit_depth > 0 {
        // Pitch is the row byte stride. Convert to pixel width:
        // scan_width = pitch * 8 / bit_depth
        (info.pitch as u32 * 8 / info.bit_depth as u32) as u16
    } else if info.width % alignment_width == 0 {
        info.width
    } else {
        alignment_width * info.width.div_ceil(alignment_width)
    };

    let expected_len = if info.bit_depth == 32 && version >= 400 {
        scan_width as usize * scan_height as usize * num_channels as usize
    } else if info.bit_depth == 1 {
        // For 1-bit: scan_width is in pixels, each row is scan_width/8 bytes
        (scan_width as usize / 8) * scan_height as usize
    } else if info.bit_depth == 2 {
        // For 2-bit: scan_width is in pixels, each row is scan_width/4 bytes
        (scan_width as usize / 4) * scan_height as usize
    } else if info.bit_depth == 4 {
        // For 4-bit: scan_width is in pixels, each row is scan_width/2 bytes
        (scan_width as usize / 2) * scan_height as usize
    } else {
        scan_width as usize * scan_height as usize * num_channels as usize
    };

    let data_was_uncompressed = reader.length >= expected_len;

    if data_was_uncompressed {
        result.extend_from_slice(&reader.data[..expected_len]);
    } else {
        while result.len() < expected_len {
            let control = match reader.read_u8() {
                Ok(v) => v as u16,
                Err(_) => break, // truncated stream is OK in Director
            };

            if control < 0x80 {
                // Literal run: copy next (control + 1) bytes
                let count = control + 1;
                for _ in 0..count {
                    if result.len() >= expected_len {
                        break;
                    }
                    match reader.read_u8() {
                        Ok(v) => result.push(v),
                        Err(_) => break,
                    }
                }
            } else if control == 0x80 {
                // No-op: skip this byte (PackBits standard)
                continue;
            } else {
                // Repeat run: repeat next byte (257 - control) times
                let count = 257 - control;
                let val = match reader.read_u8() {
                    Ok(v) => v,
                    Err(_) => break,
                };
                for _ in 0..count {
                    if result.len() >= expected_len {
                        break;
                    }
                    result.push(val);
                }
            }
        }
    }

    if info.pitch > 0 {
        // Pitch was provided — keep the pitch-based scan_width
    } else if result.len() == info.width as usize * info.height as usize * num_channels as usize {
        scan_width = info.width;
    } else if info.bit_depth == 32 && version >= 400 {
        // For 32-bit D4+ format without pitch info, use actual width (no padding)
        scan_width = info.width;
    } else if info.width % alignment_width == 0 {
        scan_width = info.width;
    } else {
        scan_width = alignment_width * info.width.div_ceil(alignment_width);
    }

    let mut bitmap = match info.bit_depth {
        1 => decode_bitmap_1bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        2 => decode_bitmap_2bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        4 => decode_bitmap_4bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        8 => decode_generic_bitmap(
            info.width,
            info.height,
            8,
            1,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
        ),
        16 => decode_bitmap_16bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
            &result,
            data_was_uncompressed,
        ),
        32 => {
            // For 32-bit bitmaps in Director, the encoding is special:
            // - Uncompressed data: direct interleaved ARGB (any version)
            // - D3 and below: always direct ARGB
            // - D4+: RLE compressed, with each scanline containing A R G B channels separately (planar)
            //
            // When `data_was_uncompressed` is true, the raw data was already the expected size
            // so no RLE decompression was applied — the data is in direct ARGB format.

            let is_direct_format = if data_was_uncompressed {
                true // Uncompressed data is always interleaved ARGB
            } else if version < 300 {
                result.len() >= (info.width as usize * info.height as usize * 4)
            } else if version < 400 {
                result.len() == (info.width as usize * info.height as usize * 4)
            } else {
                false
            };

            if is_direct_format {
                // Direct ARGB format (uncompressed data, or D3)
                let mut result_bitmap = decode_generic_bitmap(
                    info.width,
                    info.height,
                    8,
                    4,
                    scan_width,
                    scan_height,
                    PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
                    &result,
                )?;

                // Convert from ARGB (file format) to RGBA (internal format).
                // The rest of the code reads 32-bit pixels as [R, G, B, A] at each
                // 4-byte offset, but direct format stores [A, R, G, B].
                let data = &mut result_bitmap.data;
                for i in (0..data.len()).step_by(4) {
                    let a = data[i];
                    let r = data[i + 1];
                    let g = data[i + 2];
                    let b = data[i + 3];
                    data[i] = r;
                    data[i + 1] = g;
                    data[i + 2] = b;
                    data[i + 3] = a;
                }

                Ok(Bitmap {
                    width: result_bitmap.width,
                    height: result_bitmap.height,
                    bit_depth: 32,
                    original_bit_depth: 32,
                    data: result_bitmap.data,
                    palette_ref: PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
                    matte: None,
                    use_alpha: info.use_alpha,
                    trim_white_space: info.trim_white_space,
                    was_trimmed: false,
                    version: 0,
                })
            } else {
                // D4+ format: each scanline has channels laid out as A R G B sequentially
                // We need to reorder from [A...A][R...R][G...G][B...B] per line to ARGB per pixel
                let mut final_data = vec![0u8; info.width as usize * info.height as usize * 4];

                for y in 0..info.height as usize {
                    for x in 0..info.width as usize {
                        let line_offset = y * scan_width as usize * 4;
                        let pixel_idx = (y * info.width as usize + x) * 4;

                        // Check bounds
                        if line_offset + x + 3 * scan_width as usize >= result.len() {
                            web_sys::console::warn_1(&format!(
                                "32-bit decode: Out of bounds access at y={}, x={}. line_offset={}, result.len()={}",
                                y, x, line_offset, result.len()
                            ).into());
                            continue;
                        }

                        // Read from separate channels
                        let a = result[line_offset + x]; // Alpha
                        let r = result[line_offset + x + scan_width as usize]; // Red
                        let g = result[line_offset + x + 2 * scan_width as usize]; // Green
                        let b = result[line_offset + x + 3 * scan_width as usize]; // Blue

                        // Write as ARGB (or RGBA depending on your rendering system)
                        final_data[pixel_idx] = r;
                        final_data[pixel_idx + 1] = g;
                        final_data[pixel_idx + 2] = b;
                        final_data[pixel_idx + 3] = a;
                    }
                }

                Ok(Bitmap {
                    width: info.width,
                    height: info.height,
                    bit_depth: 32,
                    original_bit_depth: 32,
                    data: final_data,
                    palette_ref: PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
                    matte: None,
                    use_alpha: info.use_alpha,
                    trim_white_space: info.trim_white_space,
                    was_trimmed: false,
                    version: 0,
                })
            }
        }
        _ => Err(format!(
            "Decompression not implemented for bitmap width {}, height {}, bit depth {}",
            info.width, info.height, info.bit_depth
        )),
    }?;

    bitmap.use_alpha = info.use_alpha;
    bitmap.trim_white_space = info.trim_white_space;
    
    Ok(bitmap)
}

#[inline]
fn lookup_builtin_palette(palette: &BuiltInPalette, color_index: u8, original_bit_depth: u8) -> Option<(u8, u8, u8)> {
    match palette {
        BuiltInPalette::GrayScale => {
            // Uses 4-color palette for 2-bit images and 16-color palette for 4-bit images
            if original_bit_depth == 2 {
                GRAYSCALE_4_PALETTE.get(color_index as usize).copied()
            } else if original_bit_depth == 4 {
                GRAYSCALE_16_PALETTE.get(color_index as usize).copied()
            } else {
                GRAYSCALE_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::SystemMac => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                MAC_16_PALETTE.get(color_index as usize).copied()
            } else {
                SYSTEM_MAC_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::SystemWin => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                WIN_16_PALETTE.get(color_index as usize).copied()
            } else {
                SYSTEM_WIN_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Rainbow => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                RAINBOW16_PALETTE.get(color_index as usize).copied()
            } else {
                RAINBOW_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Pastels => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                PASTELS16_PALETTE.get(color_index as usize).copied()
            } else {
                PASTELS_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Vivid => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                VIVID16_PALETTE.get(color_index as usize).copied()
            } else {
                VIVID_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Ntsc => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                NTSC16_PALETTE.get(color_index as usize).copied()
            } else {
                NTSC_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Metallic => {
            // Use 16-color palette for 4-bit images
            if original_bit_depth == 4 {
                METALLIC16_PALETTE.get(color_index as usize).copied()
            } else {
                METALLIC_PALETTE.get(color_index as usize).copied()
            }
        }
        BuiltInPalette::Web216 => WEB_216_PALETTE.get(color_index as usize).copied(),
        // Vga and SystemWinDir4 fall back to SystemWin palette
        BuiltInPalette::Vga | BuiltInPalette::SystemWinDir4 => {
            if original_bit_depth == 4 {
                WIN_16_PALETTE.get(color_index as usize).copied()
            } else {
                SYSTEM_WIN_PALETTE.get(color_index as usize).copied()
            }
        }
    }
}

#[inline]
fn color_fallback(color_index: u8) -> (u8, u8, u8) {
    if color_index == 0 {
        (255, 255, 255)
    } else if color_index == 255 {
        (0, 0, 0)
    } else {
        (255, 0, 255) // magenta for missing colors
    }
}

#[inline]
pub fn resolve_color_ref(
    palettes: &PaletteMap,
    color_ref: &ColorRef,
    palette_ref: &PaletteRef,
    original_bit_depth: u8,
) -> (u8, u8, u8) {
    match color_ref {
        ColorRef::Rgb(r, g, b) => (*r, *g, *b),
        ColorRef::PaletteIndex(color_index) => {
            let idx = *color_index;
            match palette_ref {
                PaletteRef::BuiltIn(palette) => {
                    lookup_builtin_palette(palette, idx, original_bit_depth)
                        .unwrap_or_else(|| color_fallback(idx))
                }
                PaletteRef::Member(member_ref) => {
                    // cast_lib 0 = search all cast libs by member number
                    let palette_member = if member_ref.cast_lib == 0 {
                        palettes.find_by_member(member_ref.cast_member as u32)
                    } else {
                        let slot_number = CastMemberRefHandlers::get_cast_slot_number(
                            member_ref.cast_lib as u32,
                            member_ref.cast_member as u32,
                        );
                        palettes.get(slot_number as usize)
                            .or_else(|| palettes.find_by_member(member_ref.cast_member as u32))
                    };
                    if let Some(member) = palette_member {
                        member.colors.get(idx as usize).copied()
                            .unwrap_or_else(|| color_fallback(idx))
                    } else if let Some(member) = palettes.find_by_cast_lib(member_ref.cast_lib as u32) {
                        // Fallback: exact palette member not found (stale clutId from old numbering),
                        // use any palette in the same cast library
                        member.colors.get(idx as usize).copied()
                            .unwrap_or_else(|| color_fallback(idx))
                    } else {
                        lookup_builtin_palette(&get_system_default_palette(), idx, original_bit_depth)
                            .unwrap_or_else(|| color_fallback(idx))
                    }
                }
                PaletteRef::Default => {
                    // palette_id=0 means "no specific palette set" - use system default palette
                    lookup_builtin_palette(&get_system_default_palette(), idx, original_bit_depth)
                        .unwrap_or_else(|| color_fallback(idx))
                }
            }
        }
    }
}

/// Pre-resolve a full palette into an RGB lookup table.
/// For 4-bit bitmaps returns 16 entries, for 8-bit returns 256 entries.
/// This avoids calling `resolve_color_ref` per-pixel in hot loops.
#[inline]
pub fn resolve_palette_table(
    palettes: &PaletteMap,
    palette_ref: &PaletteRef,
    original_bit_depth: u8,
) -> Vec<(u8, u8, u8)> {
    // Always resolve all 256 entries. The internal bit_depth may be wider
    // than original_bit_depth (e.g. 1-bit stored as 8-bit), so
    // get_pixel_color_ref can return any PaletteIndex in 0..255.
    let mut table = Vec::with_capacity(256);
    for i in 0..256u16 {
        table.push(resolve_color_ref(
            palettes,
            &ColorRef::PaletteIndex(i as u8),
            palette_ref,
            original_bit_depth,
        ));
    }
    table
}

/// Decompress PackBits/RLE-compressed alpha data with even-padded row width.
/// Director stores alpha rows padded to 2-byte boundaries; without accounting for
/// this, odd-width bitmaps get a cumulative 1-byte-per-row diagonal shear.
fn decompress_alpha_rle(data: &[u8], width: usize, height: usize) -> Vec<u8> {
    let padded_width = (width + 1) & !1;
    let pixel_count = width * height;
    let padded_total = padded_width * height;
    let mut result = Vec::with_capacity(padded_total);
    let mut pos = 0;
    while result.len() < padded_total && pos < data.len() {
        let control = data[pos] as u16;
        pos += 1;

        if control < 0x80 {
            let count = (control + 1) as usize;
            for _ in 0..count {
                if result.len() >= padded_total || pos >= data.len() {
                    break;
                }
                result.push(data[pos]);
                pos += 1;
            }
        } else if control == 0x80 {
            continue;
        } else {
            let count = (257 - control) as usize;
            if pos >= data.len() { break; }
            let val = data[pos];
            pos += 1;
            for _ in 0..count {
                if result.len() >= padded_total {
                    break;
                }
                result.push(val);
            }
        }
    }

    // Strip row padding if needed
    if padded_width > width {
        let mut stripped = Vec::with_capacity(pixel_count);
        for row in 0..height {
            let row_start = row * padded_width;
            let row_end = row_start + width;
            if row_end <= result.len() {
                stripped.extend_from_slice(&result[row_start..row_end]);
            }
        }
        stripped
    } else {
        result
    }
}

/// Decode a JPEG-compressed BITD chunk. The BITD data starts with a JPEG stream (RGB),
/// optionally followed by a separate alpha channel after the JPEG end marker (FFD9).
fn decode_jpeg_bitd(data: &[u8], info: &BitmapInfo, cast_lib: u32) -> Result<Bitmap, String> {
    use image::ImageDecoder;
    use std::io::Cursor;

    // Find the end of the JPEG stream (last FFD9 marker)
    let mut jpeg_end_pos = data.len();
    for i in (0..data.len().saturating_sub(1)).rev() {
        if data[i] == 0xFF && data[i + 1] == 0xD9 {
            jpeg_end_pos = i + 2;
            break;
        }
    }

    let jpeg_data = &data[..jpeg_end_pos];
    let alpha_data = &data[jpeg_end_pos..];

    // Decode the JPEG
    let cursor = Cursor::new(jpeg_data);
    let decoder = image::codecs::jpeg::JpegDecoder::new(cursor)
        .map_err(|e| format!("Failed to create JPEG decoder for BITD: {}", e))?;

    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();

    let mut image_data = vec![0u8; decoder.total_bytes() as usize];
    decoder
        .read_image(&mut image_data)
        .map_err(|e| format!("Failed to read JPEG image from BITD: {}", e))?;

    let pixel_count = width as usize * height as usize;

    // Convert to RGBA, incorporating separate alpha channel if available
    let mut rgba_data = Vec::with_capacity(pixel_count * 4);

    // Decompress alpha data if present (may be PackBits/RLE compressed)
    let alpha_bytes = if !alpha_data.is_empty() {
        let mut alpha_result = decompress_alpha_rle(alpha_data, width as usize, height as usize);

        // If RLE didn't expand to expected size, try raw
        if alpha_result.len() < pixel_count && alpha_data.len() >= pixel_count {
            alpha_result = alpha_data[..pixel_count].to_vec();
        }

        Some(alpha_result)
    } else {
        None
    };

    match color_type {
        image::ColorType::Rgb8 => {
            for (i, chunk) in image_data.chunks(3).enumerate() {
                rgba_data.push(chunk[0]);
                rgba_data.push(chunk[1]);
                rgba_data.push(chunk[2]);
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba_data.push(alpha);
            }
        }
        image::ColorType::L8 => {
            for (i, &gray) in image_data.iter().enumerate() {
                rgba_data.push(gray);
                rgba_data.push(gray);
                rgba_data.push(gray);
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba_data.push(alpha);
            }
        }
        _ => {
            return Err(format!("Unsupported JPEG color type in BITD: {:?}", color_type));
        }
    }

    Ok(Bitmap {
        width: width as u16,
        height: height as u16,
        bit_depth: 32,
        original_bit_depth: 32,
        data: rgba_data,
        palette_ref: PaletteRef::from(info.palette_id, info.clut_cast_lib, cast_lib),
        matte: None,
        use_alpha: info.use_alpha,
        trim_white_space: info.trim_white_space,
        was_trimmed: false,
        version: 0,
    })
}

pub fn decode_jpeg_bitmap(data: &[u8], info: &BitmapInfo, alfa_data: Option<&Vec<u8>>) -> Result<Bitmap, String> {
    use image::ImageDecoder;
    use std::io::Cursor;

    // Use the `image` crate to decode JPEG
    let cursor = Cursor::new(data);
    let decoder = image::codecs::jpeg::JpegDecoder::new(cursor)
        .map_err(|e| format!("Failed to create JPEG decoder: {}", e))?;

    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();
    let pixel_count = (width * height) as usize;

    let mut image_data = vec![0u8; decoder.total_bytes() as usize];
    decoder
        .read_image(&mut image_data)
        .map_err(|e| format!("Failed to read JPEG image: {}", e))?;

    // Decompress ALFA chunk data if present (PackBits/RLE compressed)
    let alpha_bytes = if let Some(raw_alfa) = alfa_data {
        let mut alpha_result = decompress_alpha_rle(raw_alfa, width as usize, height as usize);

        // If RLE didn't produce enough bytes, try treating as raw uncompressed
        if alpha_result.len() < pixel_count && raw_alfa.len() >= pixel_count {
            alpha_result = raw_alfa[..pixel_count].to_vec();
        }

        Some(alpha_result)
    } else {
        None
    };

    let has_alfa = alpha_bytes.is_some();

    // Convert to RGBA, incorporating ALFA channel if available
    let rgba_data = match color_type {
        image::ColorType::Rgb8 => {
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for (i, chunk) in image_data.chunks(3).enumerate() {
                rgba.push(chunk[0]); // R
                rgba.push(chunk[1]); // G
                rgba.push(chunk[2]); // B
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba.push(alpha);
            }
            rgba
        }
        image::ColorType::L8 => {
            let mut rgba = Vec::with_capacity(pixel_count * 4);
            for (i, &gray) in image_data.iter().enumerate() {
                rgba.push(gray);
                rgba.push(gray);
                rgba.push(gray);
                let alpha = alpha_bytes.as_ref()
                    .and_then(|ab| ab.get(i).copied())
                    .unwrap_or(255);
                rgba.push(alpha);
            }
            rgba
        }
        _ => {
            return Err(format!("Unsupported JPEG color type: {:?}", color_type));
        }
    };

    Ok(Bitmap {
        width: width as u16,
        height: height as u16,
        bit_depth: 32,
        original_bit_depth: 32,
        data: rgba_data,
        palette_ref: PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
        matte: None,
        use_alpha: if has_alfa { info.use_alpha } else { false },
        trim_white_space: info.trim_white_space,
        was_trimmed: false,
        version: 0,
    })
}
