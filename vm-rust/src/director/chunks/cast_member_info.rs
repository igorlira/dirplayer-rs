use std::io::Error;
use binary_reader::BinaryReader;

use crate::io::list_readers::{read_pascal_string, read_string};
use crate::io::reader::DirectorExt;
use super::list::BasicListChunk;

#[allow(dead_code)]
pub struct CastMemberInfoChunkHeader {
    data_offset: usize,
    unk1: u32,
    unk2: u32,
    pub flags: u32,
    pub script_id: u32,
}

pub struct CastMemberInfoChunk {
    pub header: CastMemberInfoChunkHeader,
    pub script_src_text: String,
    pub name: String,
}

impl CastMemberInfoChunk {
    pub fn read(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<CastMemberInfoChunk, Error> {
        let header = Self::read_header(reader, dir_version)?;
        let offset_table =
            BasicListChunk::read_offset_table(reader, dir_version, header.data_offset)?;
        let item_bufs =
            BasicListChunk::read_items(reader, dir_version, header.data_offset, &offset_table)
                ?;

        let script_src_text = read_string(&item_bufs, 0);
        let name = read_pascal_string(&item_bufs, 1, reader.endian);
        // TODO Workaround: Increase table len to have at least one entry for decompilation results

        return Ok(CastMemberInfoChunk {
            header,
            script_src_text: script_src_text,
            name: name,
        });
    }

    #[allow(unused_variables)]
    fn read_header(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<CastMemberInfoChunkHeader, Error> {
        return Ok(CastMemberInfoChunkHeader {
            data_offset: reader.read_usize32()?,
            unk1: reader.read_u32()?,
            unk2: reader.read_u32()?,
            flags: reader.read_u32()?,
            script_id: reader.read_u32()?,
        });
    }
}
