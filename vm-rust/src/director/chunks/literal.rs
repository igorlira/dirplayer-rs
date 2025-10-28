use binary_reader::BinaryReader;
use num_derive::FromPrimitive;

use crate::{director::lingo::datum::Datum, io::reader::DirectorExt};

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
    Unknown1 = 11,
}

impl LiteralStore {
    #[allow(dead_code)]
    pub fn from_reader(
        reader: &mut BinaryReader,
        dir_version: u16,
        start_offset: usize,
    ) -> Result<LiteralStore, String> {
        let record = Self::read_record(reader, dir_version).unwrap();
        let data = Self::read_data(reader, &record, start_offset).unwrap();
        return Ok(LiteralStore { record, data });
    }

    pub fn read_record(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<LiteralStoreRecord, String> {
        let literal_type_id = if dir_version >= 500 {
            reader.read_u32().unwrap()
        } else {
            reader.read_u16().unwrap() as u32
        };
        let literal_type: Option<LiteralType> = num::FromPrimitive::from_u32(literal_type_id);
        let literal_type = match literal_type {
            Some(literal_type) => literal_type,
            None => return Err(format!("Invalid literal type: {}", literal_type_id)),
        };
        let offset = reader.read_u32().unwrap() as usize;
        return Ok(LiteralStoreRecord {
            literal_type,
            offset,
        });
    }

    pub fn read_data(
        reader: &mut BinaryReader,
        record: &LiteralStoreRecord,
        start_offset: usize,
    ) -> Result<Datum, String> {
        let value: Datum;
        match record.literal_type {
            LiteralType::Int => {
                value = Datum::Int(record.offset as i32);
            }
            _ => {
                reader.jmp(start_offset + record.offset);
                let length = reader.read_u32().unwrap() as usize;
                match record.literal_type {
                    LiteralType::String => {
                        value = Datum::String(reader.read_string(length - 1).unwrap());
                    }
                    LiteralType::Float => {
                        let float_val = if length == 8 {
                            // Length 8 means f64 (double precision)
                            let bytes = reader.read_bytes(8).unwrap();
                            let val_f64 = f64::from_be_bytes([
                                bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5],
                                bytes[6], bytes[7],
                            ]);
                            let val = val_f64 as f32;
                            val
                        } else if length == 10 {
                            // Apple 80-bit extended precision
                            let val = reader.read_apple_float_80().unwrap() as f32;
                            val
                        } else {
                            0.0
                        };
                        value = Datum::Float(float_val);
                    }
                    _ => {
                        value = Datum::Void;
                    }
                }
            }
        }
        return Ok(value);
    }
}
