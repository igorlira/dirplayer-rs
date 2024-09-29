use binary_reader::BinaryReader;
use log::warn;
use num_derive::FromPrimitive;

use crate::io::reader::DirectorExt;


#[derive(Copy, Clone, FromPrimitive)]
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
  Font = (15)
}

impl MemberType {
  pub fn from(val: u32) -> MemberType {
    return num::FromPrimitive::from_u32(val).unwrap();
  }
}

#[derive(Copy, Clone, FromPrimitive, PartialEq)]
pub enum ScriptType {
	Invalid = (0),
	Score = (1),
	Movie = (3),
	Parent = (7)
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

impl ScriptType {
  pub fn from(val: u16) -> ScriptType {
    return num::FromPrimitive::from_u16(val).unwrap();
  }
}

impl From<&[u8]> for BitmapInfo {
	fn from(bytes: &[u8]) -> BitmapInfo {
		let mut reader = BinaryReader::from_u8(bytes);
		reader.set_endian(binary_reader::Endian::Big);

		reader.read_u8().unwrap();
		reader.read_u8().unwrap(); // Logo -> 16
		reader.read_u32().unwrap();
		let height = reader.read_u16().unwrap();
		let width = reader.read_u16().unwrap();
		reader.read_u16().unwrap();
		reader.read_u16().unwrap();
		reader.read_u16().unwrap();
		reader.read_u16().unwrap();
		let reg_y = reader.read_i16().unwrap();
		let reg_x = reader.read_i16().unwrap();
		reader.read_u8().unwrap();
		let bit_depth;
		let palette_id;
		if reader.eof() {
			bit_depth = 1;
			palette_id = 0;
		} else {
			bit_depth = reader.read_u8().unwrap();
			reader.read_i16().unwrap(); // palette?
			palette_id = reader.read_i16().unwrap() - 1; // TODO why -1?
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

		let shape_type = reader.read_u16().unwrap(); // 00 01
		let reg_y = reader.read_u16().unwrap(); // 00 00
		let reg_x = reader.read_u16().unwrap(); // 00 00
		let height = reader.read_u16().unwrap(); // 00 36
		let width = reader.read_u16().unwrap(); // 02 d0
		let _ = reader.read_u16().unwrap();
		let color = reader.read_u8().unwrap();
		let _ = reader.read_u16().unwrap();
		let _ = reader.read_u16().unwrap();
		
		return ShapeInfo {
			shape_type: match shape_type {
				0x0001 => ShapeType::Rect,
				_ => {
					warn!("Unknown shape type: {:x}", shape_type);
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
		
		let reg_y = reader.read_u16().unwrap();
		let reg_x = reader.read_u16().unwrap();
		let height = reader.read_u16().unwrap();
		let width = reader.read_u16().unwrap();

		let _unk0 = reader.read_u24().unwrap(); // typically all zeroes

		let flags = reader.read_u8().unwrap();
		let center = flags & 0b1;
		let crop = 1 - ((flags & 0b10) >> 1);
		let sound = (flags & 0b1000) >> 3;
		let loops = 1 - ((flags & 0b100000) >> 5);
		// log_i(format_args!("FilmLoopInfo {reg_y} {reg_x} {height} {width} center={center} crop={crop} sound={sound} loop={loops}").to_string().as_str());

		// believe these bitfields are only for other cast member types
		let _unk1 = reader.read_u16().unwrap();
		
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