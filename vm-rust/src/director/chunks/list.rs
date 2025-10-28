use binary_reader::BinaryReader;

pub trait ListChunk<H, I> {
    fn from_reader(reader: &mut BinaryReader, dir_version: u16) -> Result<Vec<I>, String>;
    // fn read_header(reader: &mut BinaryReader, dir_version: u32) -> Result<H, String>;
    // fn read_offset_table(reader: &mut BinaryReader, dir_version: u32, header: H) -> Result<Vec<usize>, String>;
    // fn read_items(reader: &mut BinaryReader, dir_version: u32, header: H, offset_table: Vec<usize>) -> Result<Vec<I>, String>;
}

pub struct BasicListChunk {}

impl BasicListChunk {
    pub fn read_header(reader: &mut BinaryReader, _dir_version: u16) -> Result<usize, String> {
        let data_offset = reader.read_u32().unwrap();
        return Ok(data_offset as usize);
    }

    pub fn read_offset_table(
        reader: &mut BinaryReader,
        _: u16,
        header: usize,
    ) -> Result<Vec<usize>, String> {
        let data_offset = header;

        reader.jmp(data_offset);
        let offset_table_len = reader.read_u16().unwrap();
        let offset_table = (0..offset_table_len)
            .map(|_| reader.read_u32().unwrap() as usize)
            .collect();

        return Ok(offset_table);
    }

    #[allow(unused_variables)]
    pub fn read_items(
        reader: &mut BinaryReader,
        dir_version: u16,
        header: usize,
        offset_table: &Vec<usize>,
    ) -> Result<Vec<Vec<u8>>, String> {
        let items_len = reader.read_u32().unwrap();

        let item_endian = reader.endian;
        let list_offset = reader.pos;

        let items = (0..offset_table.len())
            .map(|i| {
                let offset = offset_table[i];
                let next_offset = if i == offset_table.len() - 1 {
                    items_len as usize
                } else {
                    offset_table[i + 1]
                };
                reader.jmp(list_offset + offset);

                return reader.read_bytes(next_offset - offset).unwrap().to_vec();
            })
            .collect();

        return Ok(items);
    }
}

impl ListChunk<usize, Vec<u8>> for BasicListChunk {
    fn from_reader(reader: &mut BinaryReader, dir_version: u16) -> Result<Vec<Vec<u8>>, String> {
        let header = Self::read_header(reader, dir_version).unwrap();
        let offset_table = Self::read_offset_table(reader, dir_version, header).unwrap();
        let items = Self::read_items(reader, dir_version, header, &offset_table).unwrap();

        return Ok(items);
    }
}
