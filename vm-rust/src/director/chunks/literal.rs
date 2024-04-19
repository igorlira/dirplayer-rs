use binary_reader::BinaryReader;
use num_derive::FromPrimitive;

use crate::{io::reader::DirectorExt, director::lingo::datum::Datum};

pub struct LiteralStoreRecord {
  pub literal_type: LiteralType,
  pub offset: usize,
}

pub struct LiteralStore {
  pub record: LiteralStoreRecord,
  pub data: Datum,
}

#[derive(Copy, Clone, FromPrimitive)]
pub enum LiteralType {
  Invalid = 0,
  String = 1,
  Int = 4,
  Float = 9,
}

impl LiteralStore {
  #[allow(dead_code)]
  pub fn from_reader(reader: &mut BinaryReader, dir_version: u16, start_offset: usize) -> Result<LiteralStore, String> {
    let record = Self::read_record(reader, dir_version).unwrap();
    let data = Self::read_data(reader, &record, start_offset).unwrap();
    return Ok(LiteralStore { 
      record,
      data,
    });
  }

  pub fn read_record(reader: &mut BinaryReader, dir_version: u16) -> Result<LiteralStoreRecord, String> {
    let literal_type: LiteralType;
    if dir_version >= 500 {
      literal_type = num::FromPrimitive::from_u32(reader.read_u32().unwrap()).unwrap();
    } else {
      literal_type = num::FromPrimitive::from_u16(reader.read_u16().unwrap()).unwrap();
    }
    let offset = reader.read_u32().unwrap() as usize;
    return Ok(LiteralStoreRecord {
      literal_type,
      offset
    })
  }

  pub fn read_data(
    reader: &mut BinaryReader,
    record: &LiteralStoreRecord,
    start_offset: usize,
  ) -> Result<Datum, String> {
    let value: Datum;
    match record.literal_type {
      LiteralType::Int => { value = Datum::Int(record.offset as i32); }
      _ => {
        reader.jmp(start_offset + record.offset);
        let length = reader.read_u32().unwrap() as usize;
        match record.literal_type {
          LiteralType::String => { 
            value = Datum::String(reader.read_string(length - 1).unwrap());
          }
          LiteralType::Float => {
            let float_val = if length == 8 {
              reader.read_f32().unwrap()
            } else if length == 10 {
              // TODO store as f64?
              reader.read_apple_float_80().unwrap() as f32
            } else {
              0.0
            };
            value = Datum::Float(float_val);
          }
          _ => { value = Datum::Void; }
        }
      }
    }
    return Ok(value);
  }
}
