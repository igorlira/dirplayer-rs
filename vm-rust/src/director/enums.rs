use binary_reader::{BinaryReader, Endian};
use log::{debug, error, warn};
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
    /// Cast library containing the palette. -1 or 0 means use the bitmap's own cast library.
    pub clut_cast_lib: i16,
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
    pub rect_top: i16,
    pub rect_left: i16,
    pub rect_bottom: i16,
    pub rect_right: i16,
    pub pattern: u16,
    pub fore_color: u8,
    pub back_color: u8,
    pub fill_type: u8,
    pub line_thickness: u8,
    pub line_direction: u8,
}

impl ShapeInfo {
    pub fn width(&self) -> i16 {
        self.rect_right - self.rect_left
    }
    pub fn height(&self) -> i16 {
        self.rect_bottom - self.rect_top
    }
    pub fn default_rect() -> ShapeInfo {
        ShapeInfo {
            shape_type: ShapeType::Rect,
            rect_top: 0,
            rect_left: 0,
            rect_bottom: 0,
            rect_right: 0,
            pattern: 0,
            fore_color: 0,
            back_color: 0,
            fill_type: 1,
            line_thickness: 0,
            line_direction: 0,
        }
    }
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

        // Bytes 14-21: rect in QuickDraw order (top, left, bottom, right)
        let rect_top = reader.read_i16().unwrap_or(0);
        let rect_left = reader.read_i16().unwrap_or(0);
        let rect_bottom = reader.read_i16().unwrap_or(0);
        let rect_right = reader.read_i16().unwrap_or(0);

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
        let mut clut_cast_lib: i16 = -1;
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

                // D5 (>= 500): clutCastLib (i16)
                if dir_version >= 500 {
                    if let Ok(val) = reader.read_i16() {
                        clut_cast_lib = val;
                    }
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
                if let Ok(val) = reader.read_i16() {
                    clut_cast_lib = val;
                }

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

        BitmapInfo {
            width,
            height,
            reg_x,
            reg_y,
            bit_depth,
            palette_id,
            clut_cast_lib,
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
        // D4/D5 shape specific data layout (17 bytes):
        //   shapeType(u16) | rect top(i16) left(i16) bottom(i16) right(i16) |
        //   pattern(u16) | fgCol(u8) | bgCol(u8) | fillType(u8) | lineThickness(u8) | lineDirection(u8)
        //
        // Example: 00 01  00 00 00 00 00 36 02 d0  00 01  ff  00  01  01  05

        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        let shape_type_raw = reader.read_u16().unwrap_or(0);
        let rect_top = reader.read_i16().unwrap_or(0);
        let rect_left = reader.read_i16().unwrap_or(0);
        let rect_bottom = reader.read_i16().unwrap_or(0);
        let rect_right = reader.read_i16().unwrap_or(0);
        let pattern = reader.read_u16().unwrap_or(0);
        let fore_color = reader.read_u8().unwrap_or(0);
        let back_color = reader.read_u8().unwrap_or(0);
        let fill_type = reader.read_u8().unwrap_or(0);
        let line_thickness = reader.read_u8().unwrap_or(1);
        let line_direction = reader.read_u8().unwrap_or(0);

        ShapeInfo {
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
            rect_top,
            rect_left,
            rect_bottom,
            rect_right,
            pattern,
            fore_color,
            back_color,
            fill_type,
            line_thickness,
            line_direction,
        }
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

#[derive(Clone, Debug)]
pub struct Shockwave3dInfo {
    pub loops: bool,
    pub duration: u32,
    pub direct_to_stage: bool,
    pub animation_enabled: bool,
    pub preload: bool,
    pub reg_point: (i32, i32),
    pub default_rect: (i32, i32, i32, i32), // left, top, right, bottom
    pub camera_position: Option<(f32, f32, f32)>,
    pub camera_rotation: Option<(f32, f32, f32)>,
    pub bg_color: Option<(u8, u8, u8)>,
    pub ambient_color: Option<(u8, u8, u8)>,
}

impl Shockwave3dInfo {
    pub fn from(bytes: &[u8]) -> Option<Shockwave3dInfo> {
        if bytes.len() < 4 { return None; }
        let str_len = u32::from_be_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]) as usize;
        // 4 (str_len) + str_len + 4 (unknown u32) + 4 ("3DPR") + 4 (block_size) = content start
        let o = 4 + str_len + 12;
        if bytes.len() < o + 80 { return None; }

        // o+0x00: unknown (6)
        // o+0x04: loops
        // o+0x08: duration
        // o+0x0C: direct_to_stage
        // o+0x10: animation_enabled
        let loops             = u32::from_be_bytes([bytes[o+4],  bytes[o+5],  bytes[o+6],  bytes[o+7]])  != 0;
        let duration          = u32::from_be_bytes([bytes[o+8],  bytes[o+9],  bytes[o+10], bytes[o+11]]);
        let direct_to_stage   = u32::from_be_bytes([bytes[o+12], bytes[o+13], bytes[o+14], bytes[o+15]]) != 0;
        let animation_enabled = u32::from_be_bytes([bytes[o+16], bytes[o+17], bytes[o+18], bytes[o+19]]) != 0;

        // Verified offsets from hex dump (o = 0x1B):
        // reg_y  @ abs 0x56 → o+0x3B = 120
        // reg_x  @ abs 0x5A → o+0x3F = 160
        // rect   @ abs 0x5E, 0x62, 0x66, 0x6A → (left=0, top=0, bottom=240, right=320)
        // preload@ abs 0x6E → o+0x53 = 1
        if bytes.len() < o + 0x57 { return None; }
        let reg_y       = i32::from_be_bytes([bytes[o+0x3B], bytes[o+0x3C], bytes[o+0x3D], bytes[o+0x3E]]);
        let reg_x       = i32::from_be_bytes([bytes[o+0x3F], bytes[o+0x40], bytes[o+0x41], bytes[o+0x42]]);
        let rect_left   = i32::from_be_bytes([bytes[o+0x43], bytes[o+0x44], bytes[o+0x45], bytes[o+0x46]]);
        let rect_top    = i32::from_be_bytes([bytes[o+0x47], bytes[o+0x48], bytes[o+0x49], bytes[o+0x4A]]);
        let rect_bottom = i32::from_be_bytes([bytes[o+0x4B], bytes[o+0x4C], bytes[o+0x4D], bytes[o+0x4E]]);
        let rect_right  = i32::from_be_bytes([bytes[o+0x4F], bytes[o+0x50], bytes[o+0x51], bytes[o+0x52]]);
        let preload     = u32::from_be_bytes([bytes[o+0x53], bytes[o+0x54], bytes[o+0x55], bytes[o+0x56]]) != 0;

        // Parse extended properties (camera, colors) from the tail of the 3DPR block.
        // These are stored as typed records: 0x16=vector(3 floats), 0x12=color(3 u32s), 0x03=string
        let mut camera_position = None;
        let mut camera_rotation = None;
        let mut bg_color = None;
        let mut ambient_color = None;
        let mut found_view_name = false;

        // Scan for type markers after preload
        let mut scan = o + 0x57;
        while scan + 4 <= bytes.len() {
            let marker = u32::from_be_bytes([bytes[scan], bytes[scan+1], bytes[scan+2], bytes[scan+3]]);
            match marker {
                0x03 => {
                    // String: 4-byte len + chars
                    scan += 4;
                    if scan + 4 > bytes.len() { break; }
                    let slen = u32::from_be_bytes([bytes[scan], bytes[scan+1], bytes[scan+2], bytes[scan+3]]) as usize;
                    scan += 4;
                    if scan + slen > bytes.len() { break; }
                    let s: String = bytes[scan..scan+slen].iter().map(|&b| b as char).collect();
                    if s == "DefaultView" || s.ends_with("View") || s.ends_with("view") {
                        found_view_name = true;
                    }
                    scan += slen;
                }
                0x16 => {
                    // Vector: 3 BE floats + 4 extra bytes
                    scan += 4;
                    if scan + 16 > bytes.len() { break; }
                    let f1 = f32::from_bits(u32::from_be_bytes([bytes[scan], bytes[scan+1], bytes[scan+2], bytes[scan+3]]));
                    let f2 = f32::from_bits(u32::from_be_bytes([bytes[scan+4], bytes[scan+5], bytes[scan+6], bytes[scan+7]]));
                    let f3 = f32::from_bits(u32::from_be_bytes([bytes[scan+8], bytes[scan+9], bytes[scan+10], bytes[scan+11]]));
                    scan += 16; // 3 floats + 4 unknown bytes

                    if found_view_name {
                        if camera_position.is_none() {
                            camera_position = Some((f1, f2, f3));
                        } else if camera_rotation.is_none() {
                            camera_rotation = Some((f1, f2, f3));
                        }
                    }
                }
                0x12 => {
                    // Color: 3 u32 components (as bytes)
                    scan += 4;
                    if scan + 12 > bytes.len() { break; }
                    let r = u32::from_be_bytes([bytes[scan], bytes[scan+1], bytes[scan+2], bytes[scan+3]]) as u8;
                    let g = u32::from_be_bytes([bytes[scan+4], bytes[scan+5], bytes[scan+6], bytes[scan+7]]) as u8;
                    let b = u32::from_be_bytes([bytes[scan+8], bytes[scan+9], bytes[scan+10], bytes[scan+11]]) as u8;
                    scan += 12;

                    // First color after basic fields is diffuse, then bg, then ambient, then directional
                    if bg_color.is_none() {
                        // Skip diffuse color (first 0x12)
                        bg_color = Some((r, g, b)); // will be overwritten
                    } else if ambient_color.is_none() {
                        ambient_color = Some((r, g, b));
                    }
                }
                _ => {
                    scan += 4; // skip unknown marker
                }
            }
        }

        // Re-parse colors in proper order: diffuse, bg, ambient, directional
        // The hex shows: 0x12 diffuse(FF,FF,FF), 0x12 bg(FD,FD,FD), 0x12 ambient(00,00,00), 0x12 directional(FF,FF,FF)
        // Let's re-scan just for colors
        let mut colors: Vec<(u8, u8, u8)> = Vec::new();
        scan = o + 0x57;
        while scan + 16 <= bytes.len() {
            let marker = u32::from_be_bytes([bytes[scan], bytes[scan+1], bytes[scan+2], bytes[scan+3]]);
            if marker == 0x12 {
                scan += 4;
                let r = u32::from_be_bytes([bytes[scan], bytes[scan+1], bytes[scan+2], bytes[scan+3]]) as u8;
                let g = u32::from_be_bytes([bytes[scan+4], bytes[scan+5], bytes[scan+6], bytes[scan+7]]) as u8;
                let b = u32::from_be_bytes([bytes[scan+8], bytes[scan+9], bytes[scan+10], bytes[scan+11]]) as u8;
                colors.push((r, g, b));
                scan += 12;
            } else {
                scan += 4;
            }
        }
        // colors[0]=diffuse, colors[1]=bg, colors[2]=ambient, colors[3]=directional
        let bg_color = colors.get(1).copied();
        let ambient_color = colors.get(2).copied();

        Some(Shockwave3dInfo {
            loops, duration, direct_to_stage, animation_enabled, preload,
            reg_point: (reg_x, reg_y),
            default_rect: (rect_left, rect_top, rect_right, rect_bottom),
            camera_position,
            camera_rotation,
            bg_color,
            ambient_color,
        })
    }
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FlashOriginMode {
    Center = 0,
    TopLeft = 1,
    Point = 2,
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FlashPlaybackMode {
    Normal = 0,
    Fixed = 1,
    LockStep = 2,
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FlashScaleMode {
    ShowAll = 0,
    NoScale = 1,
    AutoSize = 2,
    ExactFit = 3,
    NoBorder = 4,
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FlashQuality {
    AutoHigh = 0,
    AutoMedium = 1,
    Low = 2,
    High = 3,
    AutoLow = 4,
    Medium = 5,
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FlashStreamMode {
    Frame = 0,
    Idle = 1,
    Manual = 2,
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FlashEventPassMode {
    PassAlways = 0,
    PassButton = 1,
    PassNotButton = 2,
    PassNever = 3,
}

#[derive(Clone, Debug, Copy, PartialEq)]
pub enum FlashClickMode {
    BoundingBox = 0,
    Opaque = 1,
    Object = 2,
}

#[derive(Clone, Debug)]
pub struct FlashInfo {
    // SWF-derived / cached fields
    pub reg_point: (i32, i32),        // [6],[7] - center of flash rect
    pub flash_rect: (i32, i32, i32, i32), // [8],[9],[10],[11] - left,top,right,bottom
    pub bg_color: u32,                // [13] - from SWF

    // Settings
    pub direct_to_stage: bool,        // [4]
    pub image_enabled: bool,          // [14]
    pub sound_enabled: bool,          // [15]
    pub paused_at_start: bool,        // [16]
    pub loop_enabled: bool,           // [17]
    pub scale_mode: FlashScaleMode,   // [20]
    pub stream_mode: FlashStreamMode, // [21]
    pub fixed_rate: u32,              // [22]
    pub scale: f32,                   // [23]

    // Origin / view (variable section)
    pub origin_mode: FlashOriginMode, // [26]
    pub origin_h: f32,                // [27]
    pub origin_v: f32,                // [28]
    pub view_scale: f32,              // [29]
    pub view_h: f32,                  // [31]
    pub view_v: f32,                  // [32]

    // Display settings
    pub center_reg_point: bool,       // [33]
    pub quality: FlashQuality,        // [34]
    pub is_static: bool,              // [35]
    pub buttons_enabled: bool,        // [36]
    pub actions_enabled: bool,        // [37]
    pub event_pass_mode: FlashEventPassMode, // [38]
    pub click_mode: FlashClickMode,   // [39]
    pub poster_frame: u32,            // [40]
    pub playback_mode: FlashPlaybackMode, // [41]
    pub preload: bool,                // [42]
    pub buffer_size: u32,             // [3]

    // Strings
    pub source_file_name: String,
    pub common_player: String,
}

impl FlashInfo {
    pub fn from(bytes: &[u8]) -> Option<FlashInfo> {
        if bytes.len() < 9 {
            return None;
        }
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(Endian::Big);

        // Header: u32 string_len + "flash" + u32 data_len + "FLSH" + u32 data_len2 + u32 count
        let str_len = reader.read_u32().unwrap_or(0) as usize;
        if str_len == 0 || bytes.len() < 4 + str_len + 16 {
            return None;
        }
        let name_bytes = reader.read_bytes(str_len).ok()?;
        let name = String::from_utf8_lossy(name_bytes).to_string();
        if name != "flash" {
            return None;
        }

        let _data_len = reader.read_u32().unwrap_or(0);
        let fourcc = reader.read_bytes(4).ok()?;
        if fourcc != b"FLSH" {
            return None;
        }
        let _data_len2 = reader.read_u32().unwrap_or(0);
        let _count = reader.read_u32().unwrap_or(0);

        // Now read 26 u32 values (fixed section)
        let mut u32s = [0u32; 26];
        for i in 0..26 {
            u32s[i] = reader.read_u32().unwrap_or(0);
        }

        let buffer_size = u32s[3];
        let direct_to_stage = u32s[4] != 0;
        let reg_point = (u32s[6] as i32, u32s[7] as i32);
        let flash_rect = (u32s[8] as i32, u32s[9] as i32, u32s[10] as i32, u32s[11] as i32);
        let bg_color = u32s[13];
        let image_enabled = u32s[14] != 0;
        let sound_enabled = u32s[15] != 0;
        let paused_at_start = u32s[16] != 0;
        let loop_enabled = u32s[17] != 0;
        // u32s[18], u32s[19] unknown
        let scale_mode = match u32s[20] {
            1 => FlashScaleMode::NoScale,
            2 => FlashScaleMode::AutoSize,
            3 => FlashScaleMode::ExactFit,
            4 => FlashScaleMode::NoBorder,
            _ => FlashScaleMode::ShowAll,
        };
        let stream_mode = match u32s[21] {
            1 => FlashStreamMode::Idle,
            2 => FlashStreamMode::Manual,
            _ => FlashStreamMode::Frame,
        };
        let fixed_rate = u32s[22];
        let scale = f32::from_bits(u32s[23]);

        // Variable section: read remaining u32/f32 values
        let origin_mode_val = reader.read_u32().unwrap_or(0);
        let origin_mode = match origin_mode_val {
            1 => FlashOriginMode::TopLeft,
            2 => FlashOriginMode::Point,
            _ => FlashOriginMode::Center,
        };
        let origin_h = f32::from_bits(reader.read_u32().unwrap_or(0));
        let origin_v = f32::from_bits(reader.read_u32().unwrap_or(0));
        let view_scale = f32::from_bits(reader.read_u32().unwrap_or(0));
        let _unk30 = reader.read_u32().unwrap_or(0);
        let view_h = f32::from_bits(reader.read_u32().unwrap_or(0));
        let view_v = f32::from_bits(reader.read_u32().unwrap_or(0));

        let center_reg_point = reader.read_u32().unwrap_or(0) != 0;
        let quality = match reader.read_u32().unwrap_or(3) {
            0 => FlashQuality::AutoHigh,
            1 => FlashQuality::AutoMedium,
            2 => FlashQuality::Low,
            4 => FlashQuality::AutoLow,
            5 => FlashQuality::Medium,
            _ => FlashQuality::High,
        };
        let is_static = reader.read_u32().unwrap_or(0) != 0;
        let buttons_enabled = reader.read_u32().unwrap_or(1) != 0;
        let actions_enabled = reader.read_u32().unwrap_or(1) != 0;
        let event_pass_mode = match reader.read_u32().unwrap_or(0) {
            1 => FlashEventPassMode::PassButton,
            2 => FlashEventPassMode::PassNotButton,
            3 => FlashEventPassMode::PassNever,
            _ => FlashEventPassMode::PassAlways,
        };
        let click_mode = match reader.read_u32().unwrap_or(1) {
            0 => FlashClickMode::BoundingBox,
            2 => FlashClickMode::Object,
            _ => FlashClickMode::Opaque,
        };
        let poster_frame = reader.read_u32().unwrap_or(1);
        let playback_mode = match reader.read_u32().unwrap_or(0) {
            1 => FlashPlaybackMode::Fixed,
            2 => FlashPlaybackMode::LockStep,
            _ => FlashPlaybackMode::Normal,
        };
        let preload = reader.read_u32().unwrap_or(1) != 0;

        // Skip 4 unknown u32s [43-46]
        for _ in 0..4 {
            let _ = reader.read_u32();
        }

        // Source file name (length-prefixed string)
        let src_len = reader.read_u32().unwrap_or(0) as usize;
        let source_file_name = if src_len > 0 {
            let src_bytes = reader.read_bytes(src_len).unwrap_or(&[]);
            String::from_utf8_lossy(src_bytes).to_string()
        } else {
            String::new()
        };

        // Skip u32 (observed value=2, possibly string count)
        let _ = reader.read_u32();

        // Common player (length-prefixed string)
        let cp_len = reader.read_u32().unwrap_or(0) as usize;
        let common_player = if cp_len > 0 {
            let cp_bytes = reader.read_bytes(cp_len).unwrap_or(&[]);
            String::from_utf8_lossy(cp_bytes).to_string()
        } else {
            String::new()
        };

        Some(FlashInfo {
            reg_point,
            flash_rect,
            bg_color,
            direct_to_stage,
            image_enabled,
            sound_enabled,
            paused_at_start,
            loop_enabled,
            scale_mode,
            stream_mode,
            fixed_rate,
            scale,
            origin_mode,
            origin_h,
            origin_v,
            view_scale,
            view_h,
            view_v,
            center_reg_point,
            quality,
            is_static,
            buttons_enabled,
            actions_enabled,
            event_pass_mode,
            click_mode,
            poster_frame,
            playback_mode,
            preload,
            buffer_size,
            source_file_name,
            common_player,
        })
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
    pub dont_wrap: bool,             // Offset 40-43: "don't wrap" flag (0=wrap, non-zero=don't wrap)
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
    pub camera_rotation_x: f32,      // Offset 168-171: camera rotation X
    pub camera_rotation_y: f32,      // Offset 172-175: camera rotation Y
    pub camera_rotation_z: f32,      // Offset 176-179: camera rotation Z
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
        let dont_wrap_raw = reader.read_u32().unwrap_or(0);
        let dont_wrap = dont_wrap_raw != 0;
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
        let _tex_unknown_164 = reader.read_f32().unwrap_or(0.0); // unknown field between cam pos and rot
        let camera_rotation_x = reader.read_f32().unwrap_or(0.0);
        let camera_rotation_y = reader.read_f32().unwrap_or(0.0);
        let camera_rotation_z = reader.read_f32().unwrap_or(0.0);
        let tex_unknown_180 = reader.read_u32().unwrap_or(0);
        // Offset 184: texture_member name as null-terminated string
        let tex_unknown_184 = 0;
        let mut texture_member_bytes = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            if byte == 0 { break; }
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
            dont_wrap,
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

    /// Returns true if text should word-wrap (inverted from the "don't wrap" flag)
    pub fn word_wrap(&self) -> bool {
        !self.dont_wrap
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
            error!("❌ TextMemberData: too short ({} bytes)", bytes.len());
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
        debug!("📦 Text member data length: {}", data_length);

        // Skip zeros to find actual data (36 bytes of padding)
        reader.pos += 36;

        // Read dimensions and counts
        let width = reader.read_u32().unwrap_or(0);
        let height = reader.read_u32().unwrap_or(0);
        debug!("📐 Dimensions: {}x{}", width, height);

        let count1 = reader.read_u32().unwrap_or(0);
        let size1 = reader.read_u32().unwrap_or(0);
        debug!("🔢 Format count: {}, Size: {}", count1, size1);

        let count2 = reader.read_u32().unwrap_or(0);
        let size2 = reader.read_u32().unwrap_or(0);
        debug!("🔢 Run count: {}, Size: {}", count2, size2);

        let char_count = reader.read_u32().unwrap_or(0);
        let size3 = reader.read_u32().unwrap_or(0);
        debug!("🔢 Character count: {}, Size: {}", char_count, size3);

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
            error!("❌ Expected '3TEX', got '{}'", val3_str);
            return None;
        }

        debug!("✅ Found 3TEX section");

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
        debug!("📦 3TEX section length: {}", tex_length);

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
        debug!("📝 Texture flag: '{}'", no_texture.trim_end_matches('\0'));

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
                debug!("📝 Found TEXT chunk: '{}'", text);
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
        debug!("═══════════════════════════════════");
        debug!("📄 TEXT MEMBER DATA SUMMARY");
        debug!("═══════════════════════════════════");
        debug!("Dimensions:    {}x{}", self.width, self.height);

        if let Some(ref tex) = self.tex_section {
            debug!("───────────────────────────────────");
            debug!("3TEX Section:");
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
            debug!("  BG Color ID:   {}", tex.bg_color_id);
            debug!("  Char Count:    {}", tex.char_count);
            debug!("  Line Count:    {}", tex.line_count);
            debug!("  Text Offset:   {}", tex.text_offset);
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
            debug!("  Float 1:       {}", tex.float1);
            debug!("  Float 2:       {}", tex.float2);

            if !tex.text.is_empty() {
                debug!("  Text:          '{}'", tex.text);
            } else {
                debug!("  Text:          (in child Text chunk)");
            }
        }

        debug!("═══════════════════════════════════");
    }
}
