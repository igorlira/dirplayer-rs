use binary_reader::BinaryReader;

use crate::io::list_readers::{read_pascal_string, read_string};

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
    ) -> Result<CastMemberInfoChunk, String> {
        let header = Self::read_header(reader, dir_version).unwrap();
        let offset_table =
            BasicListChunk::read_offset_table(reader, dir_version, header.data_offset).unwrap();
        let item_bufs =
            BasicListChunk::read_items(reader, dir_version, header.data_offset, &offset_table)
                .unwrap();

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
    ) -> Result<CastMemberInfoChunkHeader, String> {
        return Ok(CastMemberInfoChunkHeader {
            data_offset: reader.read_u32().unwrap() as usize,
            unk1: reader.read_u32().unwrap(),
            unk2: reader.read_u32().unwrap(),
            flags: reader.read_u32().unwrap(),
            script_id: reader.read_u32().unwrap(),
        });
    }
}
