use binary_reader::{BinaryReader, Endian};

use crate::director::{chunks::cast_member_info::CastMemberInfoChunk, enums::{BitmapInfo, FilmLoopInfo, MemberType, ScriptType, ShapeInfo}};

use super::Chunk;

pub struct CastMemberChunk {
  pub member_type: MemberType,
  pub specific_data: CastMemberSpecificData,
  pub member_info: Option<CastMemberInfoChunk>,
}

pub struct CastMemberDef {
  pub chunk: CastMemberChunk,
  pub children: Vec<Option<Chunk>>,
}

impl CastMemberChunk {
  #[allow(unused_variables, unused_assignments)]
  pub fn from_reader(reader: &mut BinaryReader, dir_version: u16) -> Result<CastMemberChunk, String> {
    reader.endian = Endian::Big;

    let mut info: Option<CastMemberInfoChunk> = None;
    let info_len: usize;
    let specific_data: Vec<u8>;
    let specific_data_len: usize;
    let member_type: MemberType;
    let mut has_flags1 = false;
    let flags1: u8;
    let specific_data_parsed;

    if dir_version >= 500 {
      member_type = MemberType::from(reader.read_u32().unwrap());
      info_len = reader.read_u32().unwrap() as usize;
      specific_data_len = reader.read_u32().unwrap() as usize;

      // info
      if info_len != 0 {
        let mut info_reader = BinaryReader::from_u8(reader.read_bytes(info_len).unwrap());
        info_reader.set_endian(reader.endian);

        info = Some(CastMemberInfoChunk::read(&mut info_reader, dir_version).unwrap());
      }

      // specific data
      let has_flags1 = false;
      specific_data = reader.read_bytes(specific_data_len).unwrap().to_vec();
    } else {
      specific_data_len = reader.read_u16().unwrap() as usize;
      info_len = reader.read_u32().unwrap() as usize;

      // these bytes are common but stored in the specific data
      let mut specific_data_left = specific_data_len;
      member_type = MemberType::from(reader.read_u8().unwrap() as u32);
      specific_data_left -= 1;
      if specific_data_left != 0 {
        has_flags1 = true;
        flags1 = reader.read_u8().unwrap();
        specific_data_left -= 1;
      } else {
        has_flags1 = false;
      }

      // specific data
      specific_data = reader.read_bytes(specific_data_left).unwrap().to_vec();

      // info
      let mut info_reader = BinaryReader::from_u8(reader.read_bytes(info_len).unwrap());
      info_reader.set_endian(reader.endian);
      if info_len != 0 {
        info = Some(CastMemberInfoChunk::read(&mut info_reader, dir_version).unwrap());
      }
    }

    let mut specific_reader = BinaryReader::from_vec(&specific_data);
    specific_reader.set_endian(reader.endian);

    match member_type {
      MemberType::Script => {
        specific_data_parsed = CastMemberSpecificData::Script(
          ScriptType::from(specific_reader.read_u16().unwrap())
        );
      }
      MemberType::Bitmap => {
        specific_data_parsed = CastMemberSpecificData::Bitmap(
          BitmapInfo::from(specific_data.as_slice())
        );
      }
      MemberType::Shape => {
        specific_data_parsed = CastMemberSpecificData::Shape(
          ShapeInfo::from(specific_data.as_slice())
        );
      }
      // a few cast member types may share the same memory format
      // including film loop, movie, digital video, and xtra
      // according to More Director Movie File Unofficial Documentation:
      // https://docs.google.com/document/d/1jDBXE4Wv1AEga-o1Wi8xtlNZY4K2fHxW2Xs8RgARrqk/edit
      MemberType::FilmLoop => {
        // film loops share the same memory structure as other cast members such as video, digital movie
        specific_data_parsed = CastMemberSpecificData::FilmLoop(
          FilmLoopInfo::from(specific_data.as_slice())
        )
      }
      _ => {
        specific_data_parsed = CastMemberSpecificData::None;
      }
    }
    
    return Ok(CastMemberChunk {
      member_type,
      specific_data: specific_data_parsed,
      member_info: info,
    })
  }
}

pub enum CastMemberSpecificData {
  Script(ScriptType),
  Bitmap(BitmapInfo),
  Shape(ShapeInfo),
  FilmLoop(FilmLoopInfo),
  None
}

impl CastMemberSpecificData {
  pub fn script_type(&self) -> Option<ScriptType> {
    if let CastMemberSpecificData::Script(script_type) = self {
      Some(*script_type)
    } else {
      None
    }
  }

  pub fn bitmap_info(&self) -> Option<&BitmapInfo> {
    if let CastMemberSpecificData::Bitmap(bitmap_info) = self {
      Some(bitmap_info)
    } else {
      None
    }
  }

  pub fn shape_info(&self) -> Option<&ShapeInfo> {
    if let CastMemberSpecificData::Shape(shape_info) = self {
      Some(shape_info)
    } else {
      None
    }
  }

  pub fn film_loop_info(&self) -> Option<&FilmLoopInfo> {
    if let CastMemberSpecificData::FilmLoop(film_loop_info) = self {
      Some(film_loop_info)
    } else {
      None
    }
  }
}
