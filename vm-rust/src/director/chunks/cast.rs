use binary_reader::BinaryReader;

use crate::io::reader::DirectorExt;

pub struct CastChunk {
    pub member_ids: Vec<u32>,
}

impl CastChunk {
    pub fn from_reader(reader: &mut BinaryReader, _dir_version: u16) -> Result<CastChunk, String> {
        reader.set_endian(binary_reader::Endian::Big);

        let mut member_ids: Vec<u32> = Vec::new();
        while !reader.eof() {
            member_ids.push(reader.read_u32().unwrap());
        }

        return Ok(CastChunk {
            member_ids: member_ids,
        });
    }
}
