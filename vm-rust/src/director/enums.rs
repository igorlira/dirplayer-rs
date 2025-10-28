use binary_reader::BinaryReader;
use log::warn;
use num_derive::FromPrimitive;

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
	Shape = (8),
	Movie = (9),
	DigitalVideo = (10),
	Script = (11),
	RTE = (12),
	Font = (15),
	Unknown = (255)
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
	Movie = (3),
	Parent = (7),
	Unknown = (255)
}

impl ScriptType {
	pub fn from(val: u16) -> ScriptType {
		num::FromPrimitive::from_u16(val).unwrap_or(ScriptType::Unknown)
	}
}

#[derive(Clone)]
pub struct BitmapInfo {
	pub width: u16,
	pub height: u16,
	pub reg_x: i16,
	pub reg_y: i16,
	pub bit_depth: u8,
	pub palette_id: i16,
}

#[derive(Clone)]
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
		
		let _ = reader.read_u8();
		let _ = reader.read_u8(); // Logo -> 16
		let _ = reader.read_u32();
		if let Ok(val) = reader.read_u16() { height = val; }
		if let Ok(val) = reader.read_u16() { width = val; }
		let _ = reader.read_u16();
		let _ = reader.read_u16();
		let _ = reader.read_u16();
		let _ = reader.read_u16();
		if let Ok(val) = reader.read_i16() { reg_y = val; }
		if let Ok(val) = reader.read_i16() { reg_x = val; }
		let _ = reader.read_u8();
		
		if !reader.eof() {
			if let Ok(val) = reader.read_u8() { bit_depth = val; }
			let _ = reader.read_i16(); // palette?
			if let Ok(val) = reader.read_i16() { palette_id = val - 1; } // TODO why -1?
		};

		return BitmapInfo {
			width,
			height,
			reg_x,
			reg_y,
			bit_depth,
			palette_id,
		}
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

		if let Ok(val) = reader.read_u16() { shape_type_raw = val; } // 00 01
		if let Ok(val) = reader.read_u16() { reg_y = val; } // 00 00
		if let Ok(val) = reader.read_u16() { reg_x = val; } // 00 00
		if let Ok(val) = reader.read_u16() { height = val; } // 00 36
		if let Ok(val) = reader.read_u16() { width = val; } // 02 d0
		let _ = reader.read_u16();
		if let Ok(val) = reader.read_u8() { color = val; }
		let _ = reader.read_u16();
		let _ = reader.read_u16();
		
		return ShapeInfo {
			shape_type: match shape_type_raw  {
				0x0001 => ShapeType::Rect,
				_ => {
					warn!("Unknown shape type: {:x}", shape_type_raw );
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
		}
	}
}

#[derive(Debug, Clone, Default)]
pub struct SoundInfo {
	pub sample_rate: u32,
	pub sample_size: u16,
	pub channels: u16,
	pub sample_count: u32,
	pub duration: u32,
	//pub compression_type: u16,
}
