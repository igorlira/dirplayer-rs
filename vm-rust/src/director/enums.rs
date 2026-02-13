use binary_reader::{BinaryReader, Endian};
use log::warn;
use num_derive::FromPrimitive;

use std::convert::TryInto;
use web_sys::console;

use crate::{io::reader::DirectorExt, utils::log_i};

#[derive(Copy, Clone, FromPrimitive, Debug)]
pub enum MemberType {
    Null = 0,
    Bitmap = (1),
    FilmLoop = (2),
    Text = (3),
    Palette = (4),
    Picture = (5),
    Sound = (6),
    Button = (7),
    Flash = (8),
    Shape = (9),
    DigitalVideo = (10),
    Script = (11),
    RTE = (12),
    Transition = (13),
    Xtra = (14),
    Ole = (15),
    Font = (16),
    Shockwave3d = (17),
    Unknown = (255),
}

impl MemberType {
    pub fn from(val: u32) -> MemberType {
        num::FromPrimitive::from_u32(val).unwrap_or(MemberType::Unknown)
    }
}

#[derive(Debug, Copy, Clone, FromPrimitive, PartialEq)]
pub enum ScriptType {
    Invalid = (0),
    Score = (1),
    Member = (2),
    Movie = (3),
    Parent = (7),
    Unknown = (255),
}

impl ScriptType {
    pub fn from(val: u16) -> ScriptType {
        num::FromPrimitive::from_u16(val).unwrap_or(ScriptType::Unknown)
    }
}

#[derive(Clone, Default)]
pub struct BitmapInfo {
    pub width: u16,
    pub height: u16,
    pub reg_x: i16,
    pub reg_y: i16,
    pub bit_depth: u8,
    pub palette_id: i16,
    pub pitch: u16,
    pub use_alpha: bool,
    pub trim_white_space: bool,
    pub center_reg_point: bool,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum ShapeType {
    Rect,
    Oval,
    OvalRect,
    Line,
    Unknown,
}

#[derive(Clone)]
pub struct ShapeInfo {
    pub shape_type: ShapeType,
    pub reg_point: (i16, i16),
    pub width: u16,
    pub height: u16,
    pub color: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BoxType {
    Adjust = 0,
    Scroll = 1,
    Fixed = 2,
    Limit = 3,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Alignment {
    Left,
    Center,
    Right,
}

#[derive(Clone, Debug)]
pub struct FieldInfo {
    pub border: u8,              // Byte 0: borderSize (0-5)
    pub margin: u8,              // Byte 1: gutterSize (0-5)
    pub box_drop_shadow: u8,     // Byte 2: boxShadow (0-5)
    pub box_type: u8,            // Byte 3: textType (0=adjust, 1=scroll, 2=fixed, 3=limit)

    pub alignment: i16,          // Bytes 4-5: textAlign (-1=right, 0=left, 1=center)

    pub bgpal_r: u16,            // Bytes 6-7: background Red (QuickDraw u16)
    pub bgpal_g: u16,            // Bytes 8-9: background Green (QuickDraw u16)
    pub bgpal_b: u16,            // Bytes 10-11: background Blue (QuickDraw u16)

    pub scroll: u16,             // Bytes 12-13: scroll position

    pub rect_left: i16,          // Bytes 14-15: initial rect left
    pub rect_top: i16,           // Bytes 16-17: initial rect top
    pub rect_right: i16,         // Bytes 18-19: initial rect right
    pub rect_bottom: i16,        // Bytes 20-21: initial rect bottom

    pub max_height: u16,         // Bytes 22-23: maximum height
    pub text_shadow: u8,         // Byte 24: text shadow
    pub flags: u8,               // Byte 25: 0x1=editable, 0x2=autoTab, 0x4=don't wrap

    pub text_height: u16,        // Bytes 26-27: actual text height
}

impl From<&[u8]> for FieldInfo {
    fn from(bytes: &[u8]) -> FieldInfo {
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        let border = reader.read_u8().unwrap_or(0);
        let margin = reader.read_u8().unwrap_or(0);
        let box_drop_shadow = reader.read_u8().unwrap_or(0);
        let box_type = reader.read_u8().unwrap_or(0);

        // Bytes 4-5: alignment as i16
        let alignment = reader.read_i16().unwrap_or(0);

        // Bytes 6-11: background palette RGB as u16
        let bgpal_r = reader.read_u16().unwrap_or(0);
        let bgpal_g = reader.read_u16().unwrap_or(0);
        let bgpal_b = reader.read_u16().unwrap_or(0);

        // Bytes 12-13: scroll as u16
        let scroll = reader.read_u16().unwrap_or(0);

        // Bytes 14-21: rect (4 × i16)
        let rect_left = reader.read_i16().unwrap_or(0);
        let rect_top = reader.read_i16().unwrap_or(0);
        let rect_right = reader.read_i16().unwrap_or(0);
        let rect_bottom = reader.read_i16().unwrap_or(0);

        // Bytes 22-23: max_height as u16
        let max_height = reader.read_u16().unwrap_or(0);

        // Byte 24: text_shadow
        let text_shadow = reader.read_u8().unwrap_or(0);

        // Byte 25: flags (0x1=editable, 0x2=autoTab, 0x4=don't wrap)
        let flags = reader.read_u8().unwrap_or(0);

        // Bytes 26-27: text_height as u16
        let text_height = reader.read_u16().unwrap_or(0);

        FieldInfo {
            border,
            margin,
            box_drop_shadow,
            box_type,
            alignment,
            bgpal_r,
            bgpal_g,
            bgpal_b,
            scroll,
            rect_left,
            rect_top,
            rect_right,
            rect_bottom,
            max_height,
            text_shadow,
            flags,
            text_height,
        }
    }
}

impl FieldInfo {
    pub fn editable(&self) -> bool {
        (self.flags & 0x01) != 0
    }
    
    pub fn auto_tab(&self) -> bool {
        (self.flags & 0x02) != 0
    }
    
    pub fn wordwrap(&self) -> bool {
        (self.flags & 0x04) == 0  // Inverted: 0=true, 1=false
    }
    
    pub fn alignment_str(&self) -> String {
        match self.alignment {
            0x0000 => "left".to_string(),
            0x0001 => "center".to_string(),
            -1 => "right".to_string(),  // 0xFFFF as i16
            _ => "left".to_string(),
        }
    }
    
    pub fn box_type_str(&self) -> String {
        match self.box_type {
            0 => "adjust".to_string(),
            1 => "scroll".to_string(),
            2 => "fixed".to_string(),
            3 => "limit".to_string(),
            _ => "adjust".to_string(),
        }
    }
    
    pub fn font_name(&self) -> &str {
        // Note: font_type was removed from FieldInfo in D4/D5 format
        // Font information comes from STXT chunk instead
        "Arial"  // Default font
    }

    /// Calculate field width from rect
    pub fn width(&self) -> u16 {
        (self.rect_right - self.rect_left).max(0) as u16
    }

    /// Calculate field height from rect
    pub fn height(&self) -> u16 {
        (self.rect_bottom - self.rect_top).max(0) as u16
    }

    /// Background color as RGB (u8, u8, u8).
    /// low byte of each QuickDraw u16.
    pub fn bg_color_rgb(&self) -> (u8, u8, u8) {
        (
            (self.bgpal_r & 0xff) as u8,
            (self.bgpal_g & 0xff) as u8,
            (self.bgpal_b & 0xff) as u8,
        )
    }
}

impl BitmapInfo {
    /// Version-aware BitmapInfo parsing.
    /// D4/D5 (version < 600) and D6+ (version >= 600) have different field layouts
    /// but share the same byte positions for pitch, initialRect, regY, regX.
    pub fn from_versioned(bytes: &[u8], dir_version: u16) -> BitmapInfo {
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        let mut width = 0u16;
        let mut height = 0u16;
        let mut reg_x = 0i16;
        let mut reg_y = 0i16;
        let mut bit_depth = 1u8;
        let mut palette_id = 0i16;
        let mut pitch = 0u16;
        let mut use_alpha = false;
        let mut trim_white_space = false;
        let mut center_reg_point = false;

        // Bytes 0-1: pitch (u16) — common to all versions
        if let Ok(val) = reader.read_u16() {
            pitch = val;
        }

        // Bytes 2-9: initialRect (top: i16, left: i16, bottom: i16, right: i16)
        let top = reader.read_i16().unwrap_or(0);
        let left = reader.read_i16().unwrap_or(0);
        let bottom = reader.read_i16().unwrap_or(0);
        let right = reader.read_i16().unwrap_or(0);
        height = (bottom - top) as u16;
        width = (right - left) as u16;

        if dir_version < 600 {
            // D4/D5: bytes 10-17 = boundingRect (8 bytes, skip)
            let _ = reader.read_u16();
            let _ = reader.read_u16();
            let _ = reader.read_u16();
            let _ = reader.read_u16();

            // Bytes 18-19: regY, bytes 20-21: regX
            if let Ok(val) = reader.read_i16() { reg_y = val; }
            if let Ok(val) = reader.read_i16() { reg_x = val; }

            // D4/D5: byte 22 is padding (NOT flags), byte 23 is bitsPerPixel
            let _ = reader.read_u8(); // padding
            if !reader.eof() {
                if let Ok(val) = reader.read_u8() {
                    bit_depth = val;
                }

                // D5 (>= 500): clutCastLib (i16) — skip
                if dir_version >= 500 {
                    let _ = reader.read_i16();
                }

                // clutId (i16)
                if let Ok(val) = reader.read_i16() {
                    if val <= 0 {
                        palette_id = val - 1;
                    } else {
                        palette_id = val;
                    }
                }
            }

            // D4/D5: pitch mask is 0x0fff
            pitch &= 0x0fff;

            if bit_depth == 0 {
                bit_depth = 1;
            }

            // D4/D5 flags come from cast member header (flags1), not from specific data.
            // The center_reg_point flag for D4 is bit 0 of flags1 (kFlagCenterRegPointD4).
            // We don't have flags1 here, so leave center_reg_point = false.
        } else {
            // D6+: bytes 10-11 = alphaThreshold(1)+padding(1) or padding(2)
            // bytes 12-13 = editVersion, bytes 14-17 = scrollPoint
            let _ = reader.read_u16();
            let _ = reader.read_u16();
            let _ = reader.read_u16();
            let _ = reader.read_u16();

            // Bytes 18-19: regY, bytes 20-21: regX
            if let Ok(val) = reader.read_i16() { reg_y = val; }
            if let Ok(val) = reader.read_i16() { reg_x = val; }

            // Byte 22: updateFlags
            if let Ok(flags) = reader.read_u8() {
                center_reg_point = (flags & 0x20) != 0;   // Bit 5: centerRegPoint
                use_alpha = (flags & 0x10) != 0;           // Bit 4
                trim_white_space = (flags & 0x80) == 0;    // Bit 7 (inverted!)
            }

            // D6+: color image flag is pitch & 0x8000
            if pitch & 0x8000 != 0 {
                pitch &= 0x3fff;

                // Byte 23: bitsPerPixel
                if let Ok(val) = reader.read_u8() {
                    bit_depth = val;
                }

                // clutCastLib (D5+ always has this, D6+ qualifies)
                let _ = reader.read_i16();

                // clutId
                if let Ok(val) = reader.read_i16() {
                    if val <= 0 {
                        palette_id = val - 1;
                    } else {
                        palette_id = val;
                    }
                }
            } else {
                // No color flag: 1-bit bitmap
                bit_depth = 1;
                pitch &= 0x3fff;
            }
        }

        // Convert reg point from canvas space to bitmap-local space
        reg_x -= left;
        reg_y -= top;

        // If centerRegPoint is enabled, calculate the centered registration point
        if center_reg_point && width > 0 && height > 0 {
            reg_x = (width / 2) as i16;
            reg_y = (height / 2) as i16;
        }

        BitmapInfo {
            width,
            height,
            reg_x,
            reg_y,
            bit_depth,
            palette_id,
            pitch,
            use_alpha,
            trim_white_space,
            center_reg_point,
        }
    }
}

impl From<&[u8]> for BitmapInfo {
    fn from(bytes: &[u8]) -> BitmapInfo {
        // Default to D6+ parsing for backward compatibility
        BitmapInfo::from_versioned(bytes, 600)
    }
}

impl From<&[u8]> for ShapeInfo {
    fn from(bytes: &[u8]) -> ShapeInfo {
        // Shape specific data: 00 01   00 00 00 00   00 36   02 d0   00 01   ff   00 01   01 05
        // Shape specific data: 00 01   00 00 00 00   01 30   01 86   00 01   22   00 01   01 05
        // Shape specific data: 00 01   00 00 00 00   00 35   02 d0   00 01   ff   00 01   01 05

        // lineSize, lineDirection, pattern, filled, shapeType, hilite, regPoint

        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        let mut shape_type_raw = 0;
        let mut reg_y = 0;
        let mut reg_x = 0;
        let mut height = 0;
        let mut width = 0;
        let mut color = 0;

        if let Ok(val) = reader.read_u16() {
            shape_type_raw = val;
        } // 00 01
        if let Ok(val) = reader.read_u16() {
            reg_y = val;
        } // 00 00
        if let Ok(val) = reader.read_u16() {
            reg_x = val;
        } // 00 00
        if let Ok(val) = reader.read_u16() {
            height = val;
        } // 00 36
        if let Ok(val) = reader.read_u16() {
            width = val;
        } // 02 d0
        let _ = reader.read_u16();
        if let Ok(val) = reader.read_u8() {
            color = val;
        }
        let _ = reader.read_u16();
        let _ = reader.read_u16();

        return ShapeInfo {
            shape_type: match shape_type_raw {
                0x0001 => ShapeType::Rect,
                0x0002 => ShapeType::OvalRect,
                0x0003 => ShapeType::Oval,
                0x0008 => ShapeType::Line,
                _ => {
                    warn!("Unknown shape type: {:x}", shape_type_raw);
                    ShapeType::Unknown
                }
            },
            reg_point: (reg_x as i16, reg_y as i16),
            width,
            height,
            color,
        };
    }
}

#[derive(Clone)]
pub struct FilmLoopInfo {
    pub reg_point: (i16, i16),
    pub width: u16,
    pub height: u16,
    pub center: u8,
    pub crop: u8,
    pub sound: u8,
    pub loops: u8, // loop is a reserved keyword in Rust
}

impl From<&[u8]> for FilmLoopInfo {
    fn from(bytes: &[u8]) -> FilmLoopInfo {
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        // based on director 7
        // Define default values to use in case of a read error
        let mut reg_y = 0;
        let mut reg_x = 0;
        let mut height = 0;
        let mut width = 0;
        let mut flags = 0;
        let mut _unk1 = 0;

        // Use `if let Ok(...)` to safely handle the reads
        if let Ok(y) = reader.read_u16() {
            reg_y = y;
        }
        if let Ok(x) = reader.read_u16() {
            reg_x = x;
        }
        if let Ok(h) = reader.read_u16() {
            height = h;
        }
        if let Ok(w) = reader.read_u16() {
            width = w;
        }
        if let Ok(f) = reader.read_u24() {
            // This is the line that was causing the panic.
            // We now safely read it and ignore the value.
        }
        if let Ok(f) = reader.read_u8() {
            flags = f;
        }
        // believe these bitfields are only for other cast member types
        if let Ok(u) = reader.read_u16() {
            _unk1 = u;
        }

        let center = flags & 0b1;
        let crop = 1 - ((flags & 0b10) >> 1);
        let sound = (flags & 0b1000) >> 3;
        let loops = 1 - ((flags & 0b100000) >> 5);
        // log_i(format_args!("FilmLoopInfo {reg_y} {reg_x} {height} {width} center={center} crop={crop} sound={sound} loop={loops}").to_string().as_str());

        return FilmLoopInfo {
            reg_point: (reg_x as i16, reg_y as i16),
            width,
            height,
            center,
            crop,
            sound,
            loops,
        };
    }
}

#[derive(Debug, Clone, Default)]
pub struct SoundInfo {
    pub sample_rate: u32,
    pub sample_size: u16,
    pub channels: u16,
    pub sample_count: u32,
    pub duration: u32,
    pub loop_enabled: bool, 
    //pub compression_type: u16,
}

#[derive(Clone, Debug)]
pub struct FontInfo {
    pub font_id: u16, // Internal font resource ID
    pub name: String, // Font name (if stored or resolved)
    pub size: u16,    // point size
    pub style: u8,    // style flags (bold/italic/etc)
}

impl From<&[u8]> for FontInfo {
    fn from(bytes: &[u8]) -> FontInfo {
        use binary_reader::{BinaryReader, Endian};
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(Endian::Big);

        let font_id = reader.read_u16().unwrap_or(0);
        let size = reader.read_u16().unwrap_or(0);
        let style = reader.read_u8().unwrap_or(0);

        FontInfo {
            font_id,
            size,
            style,
            name: String::new(),
        }
    }
}

impl FontInfo {
    /// Parse FontInfo from raw bytes with FourCC prefix
    pub fn from_raw_with_fourcc(bytes: &[u8]) -> Option<FontInfo> {
        if bytes.len() < 8 {
            return None;
        }

        use binary_reader::{BinaryReader, Endian};
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(Endian::Big);

        // Skip length field
        let _length = reader.read_u32().ok()?;

        // Read FourCC
        let fourcc_bytes = reader.read_bytes(4).ok()?;
        let fourcc = String::from_utf8_lossy(&fourcc_bytes);

        if fourcc != "font" {
            return None;
        }

        // Now parse the font data
        // Based on "00 00 00 2c" after "font", seems like another length or data field
        let data_length = reader.read_u32().ok()?;

        // Try to read font info fields
        // The structure might be different, let's try to find font_id, size, style
        let font_id = reader.read_u16().unwrap_or(0);

        // Skip some bytes and look for size
        // This is empirical - you may need to adjust based on actual data
        let size = reader.read_u16().unwrap_or(12);
        let style = reader.read_u8().unwrap_or(0);

        Some(FontInfo {
            font_id,
            size,
            style,
            name: String::new(), // Name comes from member_info, not specific_data
        })
    }

    /// Check if raw bytes look like valid font data
    pub fn looks_like_real_font_data(bytes: &[u8]) -> bool {
        if bytes.len() < 8 {
            return false;
        }

        use binary_reader::{BinaryReader, Endian};
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(Endian::Big);

        // Skip the first 4 bytes (seems to be a length field)
        if let Ok(_length) = reader.read_u32() {
            // Read the FourCC type identifier
            if let Ok(fourcc_bytes) = reader.read_bytes(4) {
                let fourcc = String::from_utf8_lossy(&fourcc_bytes);

                // Check if it's "font" type
                if fourcc == "font" {
                    return true;
                }
            }
        }

        false
    }

    /// Check if raw bytes indicate text data (not font)
    pub fn looks_like_text_data(bytes: &[u8]) -> bool {
        if bytes.len() < 8 {
            return false;
        }

        use binary_reader::{BinaryReader, Endian};
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(Endian::Big);

        // Skip the first 4 bytes
        if let Ok(_length) = reader.read_u32() {
            // Read the FourCC type identifier
            if let Ok(fourcc_bytes) = reader.read_bytes(4) {
                let fourcc = String::from_utf8_lossy(&fourcc_bytes);

                // Check if it's "text" type
                if fourcc == "text" {
                    return true;
                }
            }
        }

        false
    }

    pub fn minimal(name: &str) -> Self {
        FontInfo {
            font_id: 0,
            size: 12,
            style: 0,
            name: name.to_string(),
        }
    }

    pub fn with_default_name(mut self, name: &str) -> Self {
        if self.name.is_empty() {
            self.name = name.to_string();
        }
        self
    }
}

/// TextInfo for D6+ text member specific data (with "text" FourCC header)
/// This is different from FieldInfo which is for older D4/D5 field members
#[derive(Clone, Debug, Default)]
pub struct TextInfo {
    // Header fields
    pub fourcc_length: u32,          // Offset 0-3: typically 4
    pub fourcc: [u8; 4],             // Offset 4-7: "text"
    pub data_length: u32,            // Offset 8-11: total data length
    pub editable: bool,              // Offset 12-15: editable flag (0=false, 1=true)

    // Additional fields (offsets 16+)
    pub box_type: u32,               // Offset 16-19: 0=#adjust, 1=#scroll, 2=#fixed
    pub scroll_top: u32,             // Offset 20-23: scroll top position
    pub auto_tab: bool,              // Offset 24-27: auto tab flag (0=false, 1=true)
    pub direct_to_stage: bool,       // Offset 28-31: direct to stage flag
    pub anti_alias: bool,            // Offset 32-35: anti-alias flag (0=false, 1=true)
    pub anti_alias_threshold: u32,   // Offset 36-39: anti-alias threshold (default 14)
    pub reserved_40: u32,            // Offset 40-43
    pub reserved_44: u32,            // Offset 44-47
    pub height: u32,                 // Offset 48-51: height (17 in example)
    pub width: u32,                  // Offset 52-55: width (98 in example)
    pub kerning: bool,               // Offset 56-59: kerning flag
    pub kerning_threshold: u32,      // Offset 60-63: kerning threshold value
    pub use_hypertext_styles: bool,  // Offset 64-67: use hypertext styles flag
    pub reg_y: i32,                  // Offset 68-71: registration point Y
    pub reg_x: i32,                  // Offset 72-75: registration point X
    pub center_reg_point: bool,      // Offset 76-79: center registration point flag
    pub pre_render: u32,             // Offset 80-83: 0=#none, 1=#copyInk, 2=#otherInk
    pub save_bitmap: bool,           // Offset 84-87: save bitmap flag
    // 3TEX section starts at offset 88
    pub tex_fourcc: [u8; 4],         // Offset 88-91: "3TEX"
    pub tex_length: u32,             // Offset 92-95: 3TEX section length
    pub display_face: i32,           // Offset 96-99: displayFace bitmask (-1=all, bit0=#front, bit1=#tunnel, bit2=#back)
    pub tunnel_depth: u16,           // Offset 100-101: tunnel depth (e.g., 50, 69)
    pub tex_unknown_102: u16,        // Offset 102-103
    pub bevel_type: u32,             // Offset 104-107: 0=#none, 1=#miter, 2=#round
    pub bevel_depth: u16,            // Offset 108-109: bevel depth (e.g., 1, 3)
    pub tex_unknown_110: u16,        // Offset 110-111
    pub tex_unknown_112: u32,        // Offset 112-115
    pub smoothness: u32,             // Offset 116-119: smoothness (default 5)
    pub tex_unknown_120: u32,        // Offset 120-123
    pub display_mode: u32,           // Offset 124-127: 0=#normal, 1=#mode3d
    pub directional_preset: u32,     // Offset 128-131: 0=#none, 1=#topLeft, 2=#topCenter, 3=#topRight, 4=#middleLeft, 5=#middleCenter, 6=#middleRight, 7=#bottomLeft, 8=#bottomCenter, 9=#bottomRight
    pub texture_type: u32,           // Offset 132-135: 0=#none, 1=#default, 2=#member
    pub reflectivity: u32,           // Offset 136-139: reflectivity (default 30)
    pub directional_color: u32,      // Offset 140-143: RGB color (e.g., #777777 = 0x77777700)
    pub ambient_color: u32,          // Offset 144-147: RGB color (e.g., #666666 = 0x66666600)
    pub specular_color: u32,         // Offset 148-151: RGB color (e.g., #222222 = 0x22222200)
    pub camera_position_x: f32,      // Offset 152-155: camera position X (e.g., 48.5)
    pub camera_position_y: f32,      // Offset 156-159: camera position Y (e.g., 9.0)
    pub camera_position_z: f32,      // Offset 160-163: camera position Z (e.g., 27.36)
    pub camera_rotation_x: f32,      // Offset 164-167: camera rotation X (e.g., 0.0)
    pub camera_rotation_y: f32,      // Offset 168-171: camera rotation Y (e.g., -0.0)
    pub camera_rotation_z: f32,      // Offset 172-175: camera rotation Z (e.g., 0.0)
    pub tex_unknown_176: u32,        // Offset 176-179
    pub tex_unknown_180: u32,        // Offset 180-183
    pub tex_unknown_184: u32,        // Offset 184-187
    pub texture_member: String,      // Offset 188+: texture member reference string (e.g., "NoTexture", "(member 0 of castLib 0)")

    // Raw data for further parsing
    pub raw_data: Vec<u8>,
}

impl From<&[u8]> for TextInfo {
    fn from(bytes: &[u8]) -> TextInfo {
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        let fourcc_length = reader.read_u32().unwrap_or(0);

        let mut fourcc = [0u8; 4];
        for i in 0..4 {
            fourcc[i] = reader.read_u8().unwrap_or(0);
        }

        let data_length = reader.read_u32().unwrap_or(0);

        // Editable flag at offset 12-15
        let editable_raw = reader.read_u32().unwrap_or(0);
        let editable = editable_raw != 0;

        // Read remaining known fields
        let box_type = reader.read_u32().unwrap_or(0);
        let scroll_top = reader.read_u32().unwrap_or(0);
        let auto_tab_raw = reader.read_u32().unwrap_or(0);
        let auto_tab = auto_tab_raw != 0;
        let direct_to_stage_raw = reader.read_u32().unwrap_or(0);
        let direct_to_stage = direct_to_stage_raw != 0;
        let anti_alias_raw = reader.read_u32().unwrap_or(0);
        let anti_alias = anti_alias_raw != 0;
        let anti_alias_threshold = reader.read_u32().unwrap_or(0);
        let reserved_40 = reader.read_u32().unwrap_or(0);
        let reserved_44 = reader.read_u32().unwrap_or(0);
        let height = reader.read_u32().unwrap_or(0);
        let width = reader.read_u32().unwrap_or(0);
        let kerning_raw = reader.read_u32().unwrap_or(0);
        let kerning = kerning_raw != 0;
        let kerning_threshold = reader.read_u32().unwrap_or(0);
        let use_hypertext_styles_raw = reader.read_u32().unwrap_or(0);
        let use_hypertext_styles = use_hypertext_styles_raw != 0;
        let reg_y = reader.read_i32().unwrap_or(0);
        let reg_x = reader.read_i32().unwrap_or(0);
        let center_reg_point_raw = reader.read_u32().unwrap_or(0);
        let center_reg_point = center_reg_point_raw != 0;
        let pre_render = reader.read_u32().unwrap_or(0);
        let save_bitmap_raw = reader.read_u32().unwrap_or(0);
        let save_bitmap = save_bitmap_raw != 0;
        // 3TEX section
        let mut tex_fourcc = [0u8; 4];
        for i in 0..4 {
            tex_fourcc[i] = reader.read_u8().unwrap_or(0);
        }
        let tex_length = reader.read_u32().unwrap_or(0);
        let display_face = reader.read_i32().unwrap_or(0);
        let tunnel_depth = reader.read_u16().unwrap_or(0);
        let tex_unknown_102 = reader.read_u16().unwrap_or(0);
        let bevel_type = reader.read_u32().unwrap_or(0);
        let bevel_depth = reader.read_u16().unwrap_or(0);
        let tex_unknown_110 = reader.read_u16().unwrap_or(0);
        let tex_unknown_112 = reader.read_u32().unwrap_or(0);
        let smoothness = reader.read_u32().unwrap_or(0);
        let tex_unknown_120 = reader.read_u32().unwrap_or(0);
        let display_mode = reader.read_u32().unwrap_or(0);
        let directional_preset = reader.read_u32().unwrap_or(0);
        let texture_type = reader.read_u32().unwrap_or(0);
        let reflectivity = reader.read_u32().unwrap_or(0);
        let directional_color = reader.read_u32().unwrap_or(0);
        let ambient_color = reader.read_u32().unwrap_or(0);
        let specular_color = reader.read_u32().unwrap_or(0);
        let camera_position_x = reader.read_f32().unwrap_or(0.0);
        let camera_position_y = reader.read_f32().unwrap_or(0.0);
        let camera_position_z = reader.read_f32().unwrap_or(0.0);
        let camera_rotation_x = reader.read_f32().unwrap_or(0.0);
        let camera_rotation_y = reader.read_f32().unwrap_or(0.0);
        let camera_rotation_z = reader.read_f32().unwrap_or(0.0);
        let tex_unknown_176 = reader.read_u32().unwrap_or(0);
        let tex_unknown_180 = reader.read_u32().unwrap_or(0);
        let tex_unknown_184 = reader.read_u32().unwrap_or(0);

        // Read texture_member as null-terminated string (fixed buffer size)
        let mut texture_member_bytes = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            if byte == 0 {
                break;
            }
            texture_member_bytes.push(byte);
        }
        let texture_member = String::from_utf8_lossy(&texture_member_bytes).to_string();

        TextInfo {
            fourcc_length,
            fourcc,
            data_length,
            editable,
            box_type,
            scroll_top,
            auto_tab,
            direct_to_stage,
            anti_alias,
            anti_alias_threshold,
            reserved_40,
            reserved_44,
            height,
            width,
            kerning,
            kerning_threshold,
            use_hypertext_styles,
            reg_y,
            reg_x,
            center_reg_point,
            pre_render,
            save_bitmap,
            tex_fourcc,
            tex_length,
            display_face,
            tunnel_depth,
            tex_unknown_102,
            bevel_type,
            bevel_depth,
            tex_unknown_110,
            tex_unknown_112,
            smoothness,
            tex_unknown_120,
            display_mode,
            directional_preset,
            texture_type,
            reflectivity,
            directional_color,
            ambient_color,
            specular_color,
            camera_position_x,
            camera_position_y,
            camera_position_z,
            camera_rotation_x,
            camera_rotation_y,
            camera_rotation_z,
            tex_unknown_176,
            tex_unknown_180,
            tex_unknown_184,
            texture_member,
            raw_data: bytes.to_vec(),
        }
    }
}

impl TextInfo {
    /// Check if the bytes look like D6+ text member data (has "text" FourCC)
    pub fn looks_like_text_info(bytes: &[u8]) -> bool {
        if bytes.len() < 8 {
            return false;
        }
        // Check for "text" FourCC at offset 4
        bytes[4] == b't' && bytes[5] == b'e' && bytes[6] == b'x' && bytes[7] == b't'
    }

    /// Get the FourCC as a string
    pub fn fourcc_str(&self) -> String {
        String::from_utf8_lossy(&self.fourcc).to_string()
    }

    /// Get the box type as a Lingo symbol string
    pub fn box_type_str(&self) -> &'static str {
        match self.box_type {
            0 => "#adjust",
            1 => "#scroll",
            2 => "#fixed",
            _ => "#unknown",
        }
    }

    /// Get the display mode as a Lingo symbol string
    pub fn display_mode_str(&self) -> &'static str {
        match self.display_mode {
            0 => "#normal",
            1 => "#mode3d",
            _ => "#unknown",
        }
    }

    /// Get the pre-render mode as a Lingo symbol string
    pub fn pre_render_str(&self) -> &'static str {
        match self.pre_render {
            0 => "#none",
            1 => "#copyInk",
            2 => "#otherInk",
            _ => "#unknown",
        }
    }

    /// Get the 3TEX FourCC as a string
    pub fn tex_fourcc_str(&self) -> String {
        String::from_utf8_lossy(&self.tex_fourcc).to_string()
    }

    /// Get the texture type as a Lingo symbol string
    pub fn texture_type_str(&self) -> &'static str {
        match self.texture_type {
            0 => "#none",
            1 => "#default",
            2 => "#member",
            _ => "#unknown",
        }
    }

    /// Get the bevel type as a Lingo symbol string
    pub fn bevel_type_str(&self) -> &'static str {
        match self.bevel_type {
            0 => "#none",
            1 => "#miter",
            2 => "#round",
            _ => "#unknown",
        }
    }

    /// Check if a specific face is enabled in displayFace
    pub fn has_face(&self, face: &str) -> bool {
        if self.display_face == -1 {
            return true; // All faces enabled
        }
        match face {
            "#front" => (self.display_face & 1) != 0,
            "#tunnel" => (self.display_face & 2) != 0,
            "#back" => (self.display_face & 4) != 0,
            _ => false,
        }
    }

    /// Get displayFace as a list of enabled faces
    pub fn display_face_list(&self) -> Vec<&'static str> {
        if self.display_face == -1 {
            return vec!["#front", "#back", "#tunnel"];
        }
        let mut faces = Vec::new();
        if (self.display_face & 1) != 0 {
            faces.push("#front");
        }
        if (self.display_face & 4) != 0 {
            faces.push("#back");
        }
        if (self.display_face & 2) != 0 {
            faces.push("#tunnel");
        }
        faces
    }

    /// Get the directional preset as a Lingo symbol string
    pub fn directional_preset_str(&self) -> &'static str {
        match self.directional_preset {
            0 => "#none",
            1 => "#topLeft",
            2 => "#topCenter",
            3 => "#topRight",
            4 => "#middleLeft",
            5 => "#middleCenter",
            6 => "#middleRight",
            7 => "#bottomLeft",
            8 => "#bottomCenter",
            9 => "#bottomRight",
            _ => "#unknown",
        }
    }

    /// Extract RGB tuple from a color u32 (format: RR GG BB 00)
    pub fn color_to_rgb(color: u32) -> (u8, u8, u8) {
        let r = ((color >> 24) & 0xFF) as u8;
        let g = ((color >> 16) & 0xFF) as u8;
        let b = ((color >> 8) & 0xFF) as u8;
        (r, g, b)
    }

    /// Get directional color as RGB tuple
    pub fn directional_color_rgb(&self) -> (u8, u8, u8) {
        Self::color_to_rgb(self.directional_color)
    }

    /// Get ambient color as RGB tuple
    pub fn ambient_color_rgb(&self) -> (u8, u8, u8) {
        Self::color_to_rgb(self.ambient_color)
    }

    /// Get specular color as RGB tuple
    pub fn specular_color_rgb(&self) -> (u8, u8, u8) {
        Self::color_to_rgb(self.specular_color)
    }
}

#[derive(Debug, Clone)]
pub struct TextMemberData {
    pub width: u32,
    pub height: u32,
    pub tex_section: Option<TexSection>,
}

#[derive(Debug, Clone)]
pub struct TexSection {
    // Header values
    pub color_id: i32,    // -1 = FFFFFF (white)
    pub bg_color_id: i32, // Background color
    pub unknown1: u32,
    pub unknown2: u32,

    // Text properties
    pub char_count: u32,
    pub unknown3: u32,
    pub line_count: u32,
    pub unknown4: u32,
    pub unknown5: u32,
    pub unknown6: u32,
    pub unknown7: u32,
    pub text_offset: u32,

    // Color values (RGB)
    pub color1: (u8, u8, u8),
    pub color2: (u8, u8, u8),
    pub color3: (u8, u8, u8),

    // Padding and floats
    pub padding1: u32,
    pub padding2: u32,
    pub padding3: u32,
    pub float1: f32,
    pub padding4: u32,
    pub padding5: u32,
    pub padding6: u32,
    pub float2: f32,

    // Text string
    pub text: String,
}

impl TextMemberData {
    pub fn from_raw_bytes(bytes: &[u8]) -> Option<Self> {
        if bytes.len() < 8 {
            console::log_1(&format!("❌ TextMemberData: too short ({} bytes)", bytes.len()).into());
            return None;
        }

        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(Endian::Big);

        // Read outer structure
        let length = reader.read_u32().ok()?;
        let fourcc_bytes = reader.read_bytes(4).ok()?;
        let fourcc = String::from_utf8_lossy(&fourcc_bytes);

        if fourcc != "text" {
            return None;
        }

        let data_length = reader.read_u32().ok()?;
        console::log_1(&format!("📦 Text member data length: {}", data_length).into());

        // Skip zeros to find actual data (36 bytes of padding)
        reader.pos += 36;

        // Read dimensions and counts
        let width = reader.read_u32().unwrap_or(0);
        let height = reader.read_u32().unwrap_or(0);
        console::log_1(&format!("📐 Dimensions: {}x{}", width, height).into());

        let count1 = reader.read_u32().unwrap_or(0);
        let size1 = reader.read_u32().unwrap_or(0);
        console::log_1(&format!("🔢 Format count: {}, Size: {}", count1, size1).into());

        let count2 = reader.read_u32().unwrap_or(0);
        let size2 = reader.read_u32().unwrap_or(0);
        console::log_1(&format!("🔢 Run count: {}, Size: {}", count2, size2).into());

        let char_count = reader.read_u32().unwrap_or(0);
        let size3 = reader.read_u32().unwrap_or(0);
        console::log_1(&format!("🔢 Character count: {}, Size: {}", char_count, size3).into());

        // Read the remaining values before 3TEX
        let val1 = reader.read_u32().ok()?;
        let val2 = reader.read_u32().ok()?;
        let val3_bytes_slice = reader.read_bytes(4).ok()?; // mutable borrow
        let val3_bytes: [u8; 4] = val3_bytes_slice.try_into().ok()?; // copy into fixed array
        let current_pos = reader.pos; // safe now
        let val3_str = String::from_utf8_lossy(&val3_bytes);

        console::log_1(
            &format!(
                "📍 Current position: {}, next 4 bytes should be '3TEX'",
                current_pos
            )
            .into(),
        );

        if val3_str != "3TEX" {
            console::log_1(&format!("❌ Expected '3TEX', got '{}'", val3_str).into());
            return None;
        }

        console::log_1(&"✅ Found 3TEX section".into());

        // Parse the 3TEX section
        let tex_section = Self::parse_tex_section(&mut reader, char_count)?;

        Some(TextMemberData {
            width,
            height,
            tex_section: Some(tex_section),
        })
    }

    fn parse_tex_section(
        reader: &mut BinaryReader,
        expected_char_count: u32,
    ) -> Option<TexSection> {
        let tex_length = reader.read_u32().ok()?;
        console::log_1(&format!("📦 3TEX section length: {}", tex_length).into());

        // Parse header
        let color_id = reader.read_i32().ok()?;
        let bg_color_id = reader.read_i32().ok()?;
        let unknown1 = reader.read_u32().ok()?;
        let unknown2 = reader.read_u32().ok()?;
        let char_count = reader.read_u32().ok()?;
        let unknown3 = reader.read_u32().ok()?;
        let line_count = reader.read_u32().ok()?;
        let unknown4 = reader.read_u32().ok()?;
        let unknown5 = reader.read_u32().ok()?;
        let unknown6 = reader.read_u32().ok()?;
        let unknown7 = reader.read_u32().ok()?;
        let text_offset = reader.read_u32().ok()?;

        // Read RGB colors
        let color1 = (
            reader.read_u8().ok()?,
            reader.read_u8().ok()?,
            reader.read_u8().ok()?,
        );
        let color2 = (
            reader.read_u8().ok()?,
            reader.read_u8().ok()?,
            reader.read_u8().ok()?,
        );
        let color3 = (
            reader.read_u8().ok()?,
            reader.read_u8().ok()?,
            reader.read_u8().ok()?,
        );

        // Padding and floats
        let padding1 = reader.read_u32().ok()?;
        let padding2 = reader.read_u32().ok()?;
        let padding3 = reader.read_u32().ok()?;
        let float1 = f32::from_bits(reader.read_u32().ok()?);
        let padding4 = reader.read_u32().ok()?;
        let padding5 = reader.read_u32().ok()?;
        let padding6 = reader.read_u32().ok()?;
        let float2 = f32::from_bits(reader.read_u32().ok()?);

        // Texture flag (usually 'NoTexture\0')
        let no_texture_bytes = reader.read_bytes(9).ok()?;
        let no_texture = String::from_utf8_lossy(&no_texture_bytes);
        console::log_1(&format!("📝 Texture flag: '{}'", no_texture.trim_end_matches('\0')).into());

        // -----------------------------
        // Read actual text string
        // -----------------------------
        let mut text = String::new();

        // Save current reader position (start of child chunks)
        let text_start_pos = reader.pos;

        while reader.pos + 8 <= reader.data.len() {
            // Each chunk: [length:u32][type:4bytes][data]
            let chunk_len = reader.read_u32().ok()? as usize;
            let chunk_type_bytes = reader.read_bytes(4).ok()?;
            let chunk_type = String::from_utf8_lossy(&chunk_type_bytes);

            if chunk_type == "TEXT" {
                let text_bytes = reader.read_bytes(chunk_len).ok()?;
                text = String::from_utf8_lossy(text_bytes).to_string();
                console::log_1(&format!("📝 Found TEXT chunk: '{}'", text).into());
                break;
            } else {
                // Skip unknown chunk
                reader.pos += chunk_len;
            }
        }

        Some(TexSection {
            color_id,
            bg_color_id,
            unknown1,
            unknown2,
            char_count,
            unknown3,
            line_count,
            unknown4,
            unknown5,
            unknown6,
            unknown7,
            text_offset,
            color1,
            color2,
            color3,
            padding1,
            padding2,
            padding3,
            float1,
            padding4,
            padding5,
            padding6,
            float2,
            text,
        })
    }

    pub fn log_summary(&self) {
        console::log_1(&"═══════════════════════════════════".into());
        console::log_1(&"📄 TEXT MEMBER DATA SUMMARY".into());
        console::log_1(&"═══════════════════════════════════".into());
        console::log_1(&format!("Dimensions:    {}x{}", self.width, self.height).into());

        if let Some(ref tex) = self.tex_section {
            console::log_1(&"───────────────────────────────────".into());
            console::log_1(&"3TEX Section:".into());
            console::log_1(
                &format!(
                    "  Color ID:      {} {}",
                    tex.color_id,
                    if tex.color_id == -1 {
                        "(white FFFFFF)"
                    } else {
                        ""
                    }
                )
                .into(),
            );
            console::log_1(&format!("  BG Color ID:   {}", tex.bg_color_id).into());
            console::log_1(&format!("  Char Count:    {}", tex.char_count).into());
            console::log_1(&format!("  Line Count:    {}", tex.line_count).into());
            console::log_1(&format!("  Text Offset:   {}", tex.text_offset).into());
            console::log_1(
                &format!(
                    "  Color 1:       RGB({}, {}, {})",
                    tex.color1.0, tex.color1.1, tex.color1.2
                )
                .into(),
            );
            console::log_1(
                &format!(
                    "  Color 2:       RGB({}, {}, {})",
                    tex.color2.0, tex.color2.1, tex.color2.2
                )
                .into(),
            );
            console::log_1(
                &format!(
                    "  Color 3:       RGB({}, {}, {})",
                    tex.color3.0, tex.color3.1, tex.color3.2
                )
                .into(),
            );
            console::log_1(&format!("  Float 1:       {}", tex.float1).into());
            console::log_1(&format!("  Float 2:       {}", tex.float2).into());

            if !tex.text.is_empty() {
                console::log_1(&format!("  Text:          '{}'", tex.text).into());
            } else {
                console::log_1(&"  Text:          (in child Text chunk)".into());
            }
        }

        console::log_1(&"═══════════════════════════════════".into());
    }
}
