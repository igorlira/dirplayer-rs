use std::{sync::Arc, vec};

use binary_reader::BinaryReader;
use log::warn;
use num::ToPrimitive;
use num_derive::{FromPrimitive, ToPrimitive};
use std::convert::TryInto;

use crate::{
    director::enums::BitmapInfo,
    io::reader::DirectorExt,
    player::{
        cast_lib::CastMemberRef, handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
        sprite::ColorRef,
    },
};
use num::FromPrimitive;

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

#[derive(Debug, Clone)]
pub enum PaletteRef {
    BuiltIn(BuiltInPalette),
    Member(CastMemberRef),
}

impl PaletteRef {
    pub fn from(i: i16, cast_lib: u32) -> Self {
        if i < 0 {
            PaletteRef::BuiltIn(BuiltInPalette::from_i16(i).unwrap())
        } else {
            PaletteRef::Member(CastMemberRef {
                cast_lib: cast_lib as i32,
                cast_member: i as i32 + 1,
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
    // TODO check if win or mac
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
        }
    }
}

fn get_num_channels(bit_depth: u8) -> Result<u8, String> {
    match bit_depth {
        1 | 2 | 4 | 8 | 16 => Ok(1),
        32 => Ok(4),
        _ => Err("Invalid bit depth".to_string()),
    }
}

fn get_alignment_width(bit_depth: u8) -> Result<u16, String> {
    match bit_depth {
        1 | 4 | 32 => Ok(4),
        2 | 8 => Ok(2),
        16 => Ok(1),
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
            let scan_index = (y * scan_width + x) as usize;
            if x < width {
                let pixel_index = (y * width + x) as usize;
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
        original_bit_depth: 8,
        width,
        height,
        data: result,
        palette_ref,
        matte: None,
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

        let left_value = ((left_value as f32) / 3.0 * 255.0).to_u8();
        let middle_left_value = ((middle_left_value as f32) / 3.0 * 255.0).to_u8();
        let middle_right_value = ((middle_right_value as f32) / 3.0 * 255.0).to_u8();
        let right_value = ((right_value as f32) / 3.0 * 255.0).to_u8();

        decoded_data.push(left_value);
        decoded_data.push(middle_left_value);
        decoded_data.push(middle_right_value);
        decoded_data.push(right_value);
    }

    let mut result_bmp = vec![0; width as usize * height as usize];
    for y in 0..scan_height {
        for x in 0..scan_width {
            let compressed_index = (y * scan_width + x) as usize;
            if compressed_index >= decoded_data.len() {
                return Err(format!(
                    "decode_bitmap_2bit: compressed_index {} >= decoded_data.len() {}",
                    compressed_index,
                    decoded_data.len()
                ));
            }
            if x < width {
                let pixel_index = (y * width + x) as usize;
                let pixel = decoded_data[compressed_index].unwrap();
                result_bmp[pixel_index] = pixel;
            }
        }
    }

    Ok(Bitmap {
        bit_depth: 8,
        original_bit_depth: 8,
        width,
        height,
        data: result_bmp,
        palette_ref,
        matte: None,
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
            let scan_index = (y * scan_width + x) as usize;

            if scan_index >= decoded_data.len() {
                return Err(format!(
                    "decode_bitmap_4bit: scan_index {} >= decoded_data.len() {}",
                    scan_index,
                    decoded_data.len()
                ));
            }

            let pixel = decoded_data[scan_index];
            let pixel_index = (y * width + x) as usize;

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
    let bytes_per_pixel = bit_depth / 8;
    if scan_width as usize * scan_height as usize * num_channels as usize * bytes_per_pixel as usize
        != data.len()
    {
        warn!(
            "decode_generic_bitmap: Expected {} bytes, got {}",
            scan_width * scan_height * num_channels as u16 * bytes_per_pixel as u16,
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
    let mut result = Vec::new();
    let mut _current_index = 0;
    let num_channels = get_num_channels(info.bit_depth)?;
    let alignment_width = get_alignment_width(info.bit_depth)?;

    let mut reader = BinaryReader::from_u8(data);

    let scan_height = info.height;
    let mut scan_width = if info.width % alignment_width == 0 {
        info.width
    } else {
        alignment_width * info.width.div_ceil(alignment_width)
    };

    if reader.length * 8 == scan_width as usize * scan_height as usize * info.bit_depth as usize {
        // no compression
        result.append(&mut reader.data.clone());
    } else {
        while !reader.eof() {
            let mut r_len = reader.read_u8().map_err(|x| x.to_string())? as u16;
            if 0x101 - r_len > 0x7F {
                r_len += 1;
                for _ in 0..r_len {
                    let val = reader.read_u8().map_err(|x| x.to_string())?;
                    result.push(val);
                    _current_index += 1;
                }
            } else {
                r_len = 0x101 - r_len;
                let val = reader.read_u8().map_err(|x| x.to_string())?;

                for _ in 0..r_len {
                    result.push(val);
                    _current_index += 1;
                }
            }
        }
    }

    if result.len() == info.width as usize * info.height as usize * num_channels as usize {
        scan_width = info.width;
    } else if info.width % alignment_width == 0 {
        scan_width = info.width;
    } else {
        scan_width = alignment_width * info.width.div_ceil(alignment_width);
    }

    match info.bit_depth {
        1 => decode_bitmap_1bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, cast_lib),
            &result,
        ),
        2 => decode_bitmap_2bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, cast_lib),
            &result,
        ),
        4 => decode_bitmap_4bit(
            info.width,
            info.height,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, cast_lib),
            &result,
        ),
        8 => decode_generic_bitmap(
            info.width,
            info.height,
            8,
            1,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, cast_lib),
            &result,
        ),
        16 => decode_generic_bitmap(
            info.width,
            info.height,
            16,
            1,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, cast_lib),
            &result,
        ),
        32 => {
            // For 32-bit bitmaps in Director, the encoding is special:
            // - In D3 and below: ARGB pixels in a row (skipCompression = true)
            // - In D4+: RLE encoded, with each line containing A R G B channels separately

            let skip_compression = if version < 300 {
                result.len() >= (info.width as usize * info.height as usize * 4)
            } else if version < 400 {
                result.len() == (info.width as usize * info.height as usize * 4)
            } else {
                false
            };

            if skip_compression {
                // Direct ARGB format (mainly D3)
                let result_bitmap = decode_generic_bitmap(
                    info.width,
                    info.height,
                    8,
                    4,
                    scan_width,
                    scan_height,
                    PaletteRef::from(info.palette_id, cast_lib),
                    &result,
                )?;

                Ok(Bitmap {
                    width: result_bitmap.width,
                    height: result_bitmap.height,
                    bit_depth: 32,
                    original_bit_depth: 32,
                    data: result_bitmap.data,
                    palette_ref: PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
                    matte: None,
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
                            warn!(
                                "32-bit decode: Out of bounds access at y={}, x={}. line_offset={}, result.len()={}",
                                y, x, line_offset, result.len()
                            );
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
                    palette_ref: PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
                    matte: None,
                })
            }
        }
        _ => Err(format!(
            "Decompression not implemented for bitmap width {}, height {}, bit depth {}",
            info.width, info.height, info.bit_depth
        )),
    }
}

pub fn resolve_color_ref(
    palettes: &PaletteMap,
    color_ref: &ColorRef,
    palette_ref: &PaletteRef,
    original_bit_depth: u8,
) -> (u8, u8, u8) {
    match color_ref {
        ColorRef::Rgb(r, g, b) => (*r, *g, *b),
        ColorRef::PaletteIndex(color_index) => {
            let color = match palette_ref {
                PaletteRef::BuiltIn(palette) => match palette {
                    BuiltInPalette::GrayScale => {
                        // Uses 4-color palette for 2-bit images and 16-color palette for 4-bit images
                        if original_bit_depth == 2 {
                            GRAYSCALE_4_PALETTE.get(*color_index as usize).copied()
                        } else if original_bit_depth == 4 {
                            GRAYSCALE_16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            GRAYSCALE_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::SystemMac => {
                        // Use 16-color palette for 4-bit images
                        if original_bit_depth == 4 {
                            MAC_16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            SYSTEM_MAC_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::SystemWin => {
                        // Use 16-color palette for 4-bit images
                        if original_bit_depth == 4 {
                            WIN_16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            SYSTEM_WIN_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::Rainbow => {
                        // Use 16-color palette for 4-bit images
                        if original_bit_depth == 4 {
                            RAINBOW16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            RAINBOW_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::Pastels => {
                        // Use 16-color palette for 4-bit images
                        if original_bit_depth == 4 {
                            PASTELS16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            PASTELS_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::Vivid => {
                        // Use 16-color palette for 4-bit images
                        if original_bit_depth == 4 {
                            VIVID16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            VIVID_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::Ntsc => {
                        // Use 16-color palette for 4-bit images
                        if original_bit_depth == 4 {
                            NTSC16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            NTSC_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::Metallic => {
                        // Use 16-color palette for 4-bit images
                        if original_bit_depth == 4 {
                            METALLIC16_PALETTE.get(*color_index as usize).copied()
                        } else {
                            METALLIC_PALETTE.get(*color_index as usize).copied()
                        }
                    }
                    BuiltInPalette::Web216 => WEB_216_PALETTE.get(*color_index as usize).copied(),
                    _ => None,
                },
                PaletteRef::Member(palette_ref) => {
                    let palette_member = CastMemberRefHandlers::get_cast_slot_number(
                        palette_ref.cast_lib as u32,
                        palette_ref.cast_member as u32,
                    );
                    let palette_member = palettes.get(palette_member as usize);
                    match palette_member {
                        Some(member) => member.colors.get(*color_index as usize).copied(),
                        None => {
                            // If a member is not found, use the system palette
                            Some(resolve_color_ref(
                                palettes,
                                color_ref,
                                &PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
                                original_bit_depth,
                            ))
                        }
                    }
                }
            };

            if let Some(color) = color {
                return color.clone();
            } else if *color_index == 0 {
                return (255, 255, 255);
            } else if *color_index == 255 {
                return (0, 0, 0);
            } else {
                return (255, 0, 255); // magenta for missing colors
            }
        }
    }
}

pub fn decode_jpeg_bitmap(data: &[u8], info: &BitmapInfo) -> Result<Bitmap, String> {
    use image::ImageDecoder;
    use std::io::Cursor;

    // Use the `image` crate to decode JPEG
    let cursor = Cursor::new(data);
    let decoder = image::codecs::jpeg::JpegDecoder::new(cursor)
        .map_err(|e| format!("Failed to create JPEG decoder: {}", e))?;

    let (width, height) = decoder.dimensions();
    let color_type = decoder.color_type();

    let mut image_data = vec![0u8; decoder.total_bytes() as usize];
    decoder
        .read_image(&mut image_data)
        .map_err(|e| format!("Failed to read JPEG image: {}", e))?;

    // Convert to RGBA if needed
    let rgba_data = match color_type {
        image::ColorType::Rgb8 => {
            // Convert RGB to RGBA
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for chunk in image_data.chunks(3) {
                rgba.push(chunk[0]); // R
                rgba.push(chunk[1]); // G
                rgba.push(chunk[2]); // B
                rgba.push(255); // A (fully opaque)
            }
            rgba
        }
        image::ColorType::Rgba8 => {
            // Already RGBA
            image_data
        }
        image::ColorType::L8 => {
            // Grayscale to RGBA
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for &gray in &image_data {
                rgba.push(gray); // R
                rgba.push(gray); // G
                rgba.push(gray); // B
                rgba.push(255); // A
            }
            rgba
        }
        image::ColorType::La8 => {
            // Grayscale + Alpha to RGBA
            let mut rgba = Vec::with_capacity((width * height * 4) as usize);
            for chunk in image_data.chunks(2) {
                rgba.push(chunk[0]); // R
                rgba.push(chunk[0]); // G
                rgba.push(chunk[0]); // B
                rgba.push(chunk[1]); // A
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
    })
}
