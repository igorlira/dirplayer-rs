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
#[repr(C, packed)]
pub struct FieldInfo {
    pub border: u8,              // Byte 0: 0-5
    pub margin: u8,              // Byte 1: 0-5
    pub box_drop_shadow: u8,     // Byte 2: 0-5
    pub box_type: u8,            // Byte 3: 0=adjust, 1=scroll, 2=fixed, 3=limit
    
    pub alignment_high: u8,      // Byte 4
    pub alignment_low: u8,       // Byte 5
    
    pub bg_color_r: u8,          // Byte 6
    pub bg_color_g: u8,          // Byte 7
    pub bg_color_r_dup: u8,      // Byte 8 (seems duplicated?)
    pub bg_color_g_dup: u8,      // Byte 9 (seems duplicated?)
    pub fg_color_r: u8,          // Byte 10
    pub fg_color_g: u8,          // Byte 11
    
    pub reserved_12: u8,         // Byte 12: always 0x00
    pub scroll_top: u8,          // Byte 13
    
    pub reserved_14_18: [u8; 5], // Bytes 14-18: always 0x00
    
    pub reserved_19: u8,         // Byte 19: unknown purpose
    pub reserved_20: u8,         // Byte 20: always 0x00
    pub width: u8,               // Byte 21
    pub reserved_22: u8,         // Byte 22: unknown purpose
    
    pub height: u8,              // Byte 23
    pub font_type: u8,           // Byte 24
    
    pub drop_shadow: u8,         // Byte 25: 0-5
    pub flags: u8,               // Byte 26: editable|autoTab|wordwrap bits
    
    pub reserved_27_28: [u8; 2], // Bytes 27-28: always 0x00
}

impl From<&[u8]> for FieldInfo {
    fn from(bytes: &[u8]) -> FieldInfo {
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        let border = reader.read_u8().unwrap_or(0);
        let margin = reader.read_u8().unwrap_or(0);
        let box_drop_shadow = reader.read_u8().unwrap_or(0);
        let box_type = reader.read_u8().unwrap_or(0);
        
        let alignment_high = reader.read_u8().unwrap_or(0);
        let alignment_low = reader.read_u8().unwrap_or(0);
        
        let bg_color_r = reader.read_u8().unwrap_or(0);
        let bg_color_g = reader.read_u8().unwrap_or(0);
        let bg_color_r_dup = reader.read_u8().unwrap_or(0);
        let bg_color_g_dup = reader.read_u8().unwrap_or(0);
        let fg_color_r = reader.read_u8().unwrap_or(0);
        let fg_color_g = reader.read_u8().unwrap_or(0);
        
        let reserved_12 = reader.read_u8().unwrap_or(0);
        let scroll_top = reader.read_u8().unwrap_or(0);
        
        let mut reserved_14_18 = [0u8; 5];
        for i in 0..5 {
            reserved_14_18[i] = reader.read_u8().unwrap_or(0);
        }
        
        let reserved_19 = reader.read_u8().unwrap_or(0);
        let reserved_20 = reader.read_u8().unwrap_or(0);
        let width = reader.read_u8().unwrap_or(0);
        let reserved_22 = reader.read_u8().unwrap_or(0);
        
        let height = reader.read_u8().unwrap_or(0);
        let font_type = reader.read_u8().unwrap_or(0);
        
        let drop_shadow = reader.read_u8().unwrap_or(0);
        let flags = reader.read_u8().unwrap_or(0);
        
        let mut reserved_27_28 = [0u8; 2];
        reserved_27_28[0] = reader.read_u8().unwrap_or(0);
        reserved_27_28[1] = reader.read_u8().unwrap_or(0);

        FieldInfo {
            border,
            margin,
            box_drop_shadow,
            box_type,
            alignment_high,
            alignment_low,
            bg_color_r,
            bg_color_g,
            bg_color_r_dup,
            bg_color_g_dup,
            fg_color_r,
            fg_color_g,
            reserved_12,
            scroll_top,
            reserved_14_18,
            reserved_19,
            reserved_20,
            width,
            reserved_22,
            height,
            font_type,
            drop_shadow,
            flags,
            reserved_27_28,
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
        match (self.alignment_high, self.alignment_low) {
            (0x00, 0x00) => "left".to_string(),
            (0x00, 0x01) => "center".to_string(),
            (0xFF, 0xFF) => "right".to_string(),
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
        match self.font_type {
            0x10 => "Arial",
            0x0E => "Courier",
            0x11 => "Times New Roman",
            0x0F => "Calibri",
            _ => "Arial",
        }
    }
    
    // Convert RGB components to a single color value
    // This depends on your color format - adjust as needed
    pub fn bg_color(&self) -> u16 {
        // Example: Convert RGB to 16-bit color (RGB565 format)
        // Or you might need RGB888 packed into u32
        // For now, returning a placeholder
        let r = self.bg_color_r;
        let g = self.bg_color_g;
        let b = 0u8; // We still need to find where blue component is
        
        // If using RGB565: (R5 << 11) | (G6 << 5) | B5
        // If using RGB888 packed: (R8 << 16) | (G8 << 8) | B8
        // Adjust based on your actual color format
        ((r as u16) << 8) | (g as u16)
    }
    
    pub fn fg_color(&self) -> u16 {
        let r = self.fg_color_r;
        let g = self.fg_color_g;
        ((r as u16) << 8) | (g as u16)
    }
}

impl From<&[u8]> for BitmapInfo {
    fn from(bytes: &[u8]) -> BitmapInfo {
        let mut reader = BinaryReader::from_u8(bytes);
        reader.set_endian(binary_reader::Endian::Big);

        let mut width = 0;
        let mut height = 0;
        let mut reg_x = 0;
        let mut reg_y = 0;
        let mut bit_depth = 1;
        let mut palette_id = 0;
        let mut use_alpha = false;
        let mut trim_white_space = false;
        let mut center_reg_point = false;

        let _ = reader.read_u8();
        let _ = reader.read_u8(); // Logo -> 16
        let _ = reader.read_u32();
        if let Ok(val) = reader.read_u16() {
            height = val;
        }
        if let Ok(val) = reader.read_u16() {
            width = val;
        }
        let _ = reader.read_u16();
        let _ = reader.read_u16();
        let _ = reader.read_u16();
        let _ = reader.read_u16();
        if let Ok(val) = reader.read_i16() {
            reg_y = val;
        }
        if let Ok(val) = reader.read_i16() {
            reg_x = val;
        }

        // Read flags byte
        if let Ok(flags) = reader.read_u8() {
            // ScummVM: kFlagCenterRegPointD4 = 0x01 (bit 0) for D4
            // ScummVM: kFlagCenterRegPoint = 0x20 (bit 5) for D6+
            center_reg_point = (flags & 0x20) != 0;   // Bit 5: centerRegPoint (D6+ format)
            use_alpha = (flags & 0x10) != 0;           // Bit 4
            trim_white_space = (flags & 0x80) == 0;   // Bit 7 (inverted!)
        }

        if !reader.eof() {
            if let Ok(val) = reader.read_u8() {
                bit_depth = val;
            }
            let _ = reader.read_i16(); // palette?
            if let Ok(val) = reader.read_i16() {
                palette_id = val - 1;
            } // TODO why -1?
        };

        // If centerRegPoint is enabled, calculate the centered registration point
        // The raw reg_x/reg_y values need to be converted to centered coordinates
        if center_reg_point && width > 0 && height > 0 {
            reg_x = (width / 2) as i16;
            reg_y = (height / 2) as i16;
        }

        return BitmapInfo {
            width,
            height,
            reg_x,
            reg_y,
            bit_depth,
            palette_id,
            use_alpha,
            trim_white_space,
            center_reg_point,
        };
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
