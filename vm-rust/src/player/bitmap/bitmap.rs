use std::{sync::Arc, vec};

use binary_reader::BinaryReader;
use log::warn;
use num::ToPrimitive;
use num_derive::{FromPrimitive, ToPrimitive};

use crate::{
    director::enums::BitmapInfo,
    io::reader::DirectorExt,
    player::{
        cast_lib::CastMemberRef,
        handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers, sprite::ColorRef,
    },
};
use num::FromPrimitive;

use super::{mask::BitmapMask, palette::{SYSTEM_MAC_PALETTE, SYSTEM_WIN_PALETTE}, palette_map::PaletteMap};

#[derive(Clone)]
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
    pub bit_depth: u8,
    pub data: Vec<u8>, // RGBA
    pub palette_ref: PaletteRef,
    pub matte: Option<Arc<BitmapMask>>,
}

impl Bitmap {
    pub fn new(width: u16, height: u16, bit_depth: u8, palette_ref: PaletteRef) -> Self {
        let bytes_per_pixel = bit_depth as usize / 8;
        let initial_color = match bit_depth {
            16 => 255,
            32 => 255,
            _ => 0,
        };
        let data = vec![initial_color; width as usize * height as usize * bytes_per_pixel as usize];
        Self {
            width,
            height,
            bit_depth,
            data,
            palette_ref,
            matte: None,
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
                let pixel = scan_data[scan_index];
                result[pixel_index] = pixel;
            }
        }
    }

    Ok(Bitmap {
        bit_depth: 8,
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
    let mut decoded_data = Vec::new();

    for i in 0..data.len() {
        let original_value = data[i];
        let left_value = (original_value & 0xF0) >> 4;
        let right_value = original_value & 0x0F;

        decoded_data.push(left_value);
        decoded_data.push(right_value);
    }

    let mut result_bmp = Vec::new();
    for y in 0..scan_height {
        for x in 0..scan_width {
            if x >= width {
                continue;
            }
            let scan_index = (y * scan_width + x) as usize;
            let pixel_index = (y * width + x) as usize;
            let pixel = decoded_data[scan_index];

            if pixel_index % 2 == 0 {
                let new_pixel = (pixel << 4) as u16;
                result_bmp.push(new_pixel as u8);
            } else {
                let last_pixel = result_bmp.pop().unwrap();
                result_bmp.push(last_pixel | (pixel & 0xF));
            }
        }
    }

    Ok(Bitmap {
        bit_depth: 4,
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
    // TODO the 16bit parsing is broken, look into that
    let bytes_per_pixel = bit_depth / 8;
    if scan_width as usize * scan_height as usize * num_channels as usize * bytes_per_pixel as usize != data.len()
    {
        warn!(
            "decode_generic_bitmap: Expected {} bytes, got {}",
            scan_width * scan_height * num_channels as u16 * bytes_per_pixel as u16,
            data.len()
        );
        return Ok(Bitmap::new(width, height, bit_depth, palette_ref));
    } else {
        let mut result =
            vec![
                0;
                width as usize * height as usize * num_channels as usize * bytes_per_pixel as usize
            ];
        for y in 0..scan_height {
            for x in 0..scan_width {
                for c in 0..num_channels {
                    for b in 0..bytes_per_pixel {
                        let scan_index = (y as usize
                            * scan_width as usize
                            * num_channels as usize
                            * bytes_per_pixel as usize)
                            + x as usize
                            + c as usize
                            + b as usize;
                        if x < width {
                            let result_index = (y as usize
                                * width as usize
                                * num_channels as usize
                                * bytes_per_pixel as usize)
                                + x as usize
                                + c as usize
                                + b as usize;
                            result[result_index as usize] = data[scan_index as usize];
                        }
                    }
                }
            }
        }
        return Ok(Bitmap {
            width,
            height,
            bit_depth,
            data: result,
            palette_ref,
            matte: None,
        });
    }
}

// Converts a NUU-encoded bitmap to a raw bitmap
pub fn decompress_bitmap(data: &[u8], info: &BitmapInfo, cast_lib: u32) -> Result<Bitmap, String> {
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
        32 => decode_generic_bitmap(
            info.width,
            info.height,
            8,
            4,
            scan_width,
            scan_height,
            PaletteRef::from(info.palette_id, cast_lib),
            &result,
        ),
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
) -> (u8, u8, u8) {
    match color_ref {
        ColorRef::Rgb(r, g, b) => (*r, *g, *b),
        ColorRef::PaletteIndex(color_index) => {
            let color = match palette_ref {
                PaletteRef::BuiltIn(palette) => match palette {
                    BuiltInPalette::GrayScale => {
                        let value = (*color_index) as u8;
                        Some((255 - value, 255 - value, 255 - value))
                    }
                    BuiltInPalette::SystemMac => SYSTEM_MAC_PALETTE
                        .get(*color_index as usize)
                        .map(|x| x.to_owned()),
                    BuiltInPalette::SystemWin => SYSTEM_WIN_PALETTE
                        .get(*color_index as usize)
                        .map(|x| x.to_owned()),
                    _ => None,
                },
                PaletteRef::Member(palette_ref) => {
                    let palette_member = CastMemberRefHandlers::get_cast_slot_number(
                        palette_ref.cast_lib as u32,
                        palette_ref.cast_member as u32,
                    );
                    let palette_member = palettes.get(palette_member as usize);
                    palette_member
                        .and_then(|x| x.colors.get(*color_index as usize))
                        .map(|x| x.to_owned())
                }
            };

            if let Some(color) = color {
                return color.clone();
            } else if *color_index == 0 {
                return (255, 255, 255);
            } else if *color_index == 255 {
                return (0, 0, 0);
            } else {
                return (255, 0, 255);
            }
        }
    }
}
