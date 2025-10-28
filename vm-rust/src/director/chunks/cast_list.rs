use binary_reader::{BinaryReader, Endian};

use crate::io::list_readers::{read_pascal_string, read_u16};

use super::list::BasicListChunk;

pub struct CastListChunk {
    pub entries: Vec<CastListEntry>,
}

#[allow(dead_code)]
struct CastListChunkHeader {
    data_offset: usize,
    unk0: u16,
    cast_count: u16,
    items_per_cast: u16,
    unk1: u16,
}

impl CastListChunk {
    #[allow(unused_variables)]
    fn read_header(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<CastListChunkHeader, String> {
        return Ok(CastListChunkHeader {
            data_offset: reader.read_u32().unwrap() as usize,
            unk0: reader.read_u16().unwrap(),
            cast_count: reader.read_u16().unwrap(),
            items_per_cast: reader.read_u16().unwrap(),
            unk1: reader.read_u16().unwrap(),
        });
    }

    fn read_items(
        reader: &mut BinaryReader,
        dir_version: u16,
        header: CastListChunkHeader,
        offset_table: &Vec<usize>,
        item_endian: Endian,
    ) -> Result<Vec<CastListEntry>, String> {
        let item_bufs =
            BasicListChunk::read_items(reader, dir_version, header.data_offset, offset_table)
                .unwrap();
        let entries = (0..header.cast_count)
            .map(|i| {
                let mut name = "".to_string();
                let mut file_path = "".to_string();
                let mut preload_settings: u16 = 0;
                let mut min_member: u16 = 0;
                let mut max_member: u16 = 0;
                let mut id: u32 = 0;

                if header.items_per_cast >= 1 {
                    name = read_pascal_string(
                        &item_bufs,
                        (i * header.items_per_cast + 1) as usize,
                        item_endian,
                    );
                }
                if header.items_per_cast >= 2 {
                    file_path = read_pascal_string(
                        &item_bufs,
                        (i * header.items_per_cast + 2) as usize,
                        item_endian,
                    );
                }
                if header.items_per_cast >= 3 {
                    preload_settings = read_u16(
                        &item_bufs,
                        (i * header.items_per_cast + 3) as usize,
                        item_endian,
                    );
                }
                if header.items_per_cast >= 4 {
                    let mut item_reader = BinaryReader::from_vec(
                        &item_bufs[(i * header.items_per_cast + 4) as usize],
                    );
                    item_reader.set_endian(reader.endian);

                    min_member = item_reader.read_u16().unwrap();
                    max_member = item_reader.read_u16().unwrap();
                    id = item_reader.read_u32().unwrap();
                }

                return CastListEntry {
                    name: name,
                    file_path: file_path,
                    preload_settings: preload_settings,
                    min_member: min_member,
                    max_member: max_member,
                    id: id,
                };
            })
            .collect();

        return Ok(entries);
    }
}

impl CastListChunk {
    pub fn from_reader(
        reader: &mut BinaryReader,
        dir_version: u16,
        item_endian: Endian,
    ) -> Result<CastListChunk, String> {
        reader.set_endian(Endian::Big);

        let header = Self::read_header(reader, dir_version).unwrap();
        let offset_table =
            BasicListChunk::read_offset_table(reader, dir_version, header.data_offset).unwrap();
        //let item_endian = reader.endian;

        let items =
            Self::read_items(reader, dir_version, header, &offset_table, item_endian).unwrap();
        return Ok(CastListChunk { entries: items });
    }
}

pub struct CastListEntry {
    pub name: String,
    pub file_path: String,
    pub preload_settings: u16,
    pub min_member: u16,
    pub max_member: u16,
    pub id: u32,
}
