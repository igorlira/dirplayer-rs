use std::io::Error;
use binary_reader::BinaryReader;

pub trait ListChunk<H, I> {
    fn from_reader(reader: &mut BinaryReader, dir_version: u16) -> Result<Vec<I>, Error>;
    // fn read_header(reader: &mut BinaryReader, dir_version: u32) -> Result<H, String>;
    // fn read_offset_table(reader: &mut BinaryReader, dir_version: u32, header: H) -> Result<Vec<usize>, String>;
    // fn read_items(reader: &mut BinaryReader, dir_version: u32, header: H, offset_table: Vec<usize>) -> Result<Vec<I>, String>;
}

pub struct BasicListChunk {}

impl BasicListChunk {
    pub fn read_header(reader: &mut BinaryReader, _dir_version: u16) -> Result<usize, Error> {
        let data_offset = reader.read_u32()?;
        return Ok(data_offset as usize);
    }

    pub fn read_offset_table(
        reader: &mut BinaryReader,
        _: u16,
        header: usize,
    ) -> Result<Vec<usize>, Error> {
        let data_offset = header;

        reader.jmp(data_offset);
        let offset_table_len = reader.read_u16()?;
        let offset_table = (0..offset_table_len)
            .map(|_| reader.read_u32().map(|x| x as usize))
            .collect::<Result<_, _>>()?;

        return Ok(offset_table);
    }

    #[allow(unused_variables)]
    pub fn read_items(
        reader: &mut BinaryReader,
        dir_version: u16,
        header: usize,
        offset_table: &Vec<usize>,
    ) -> Result<Vec<Vec<u8>>, Error> {
        let items_len = reader.read_u32()?;

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

                return reader.read_bytes(next_offset - offset).map(|v| v.to_vec());
            })
            .collect::<Result<_, _>>()?;

        return Ok(items);
    }
}

impl ListChunk<usize, Vec<u8>> for BasicListChunk {
    fn from_reader(reader: &mut BinaryReader, dir_version: u16) -> Result<Vec<Vec<u8>>, Error> {
        let header = Self::read_header(reader, dir_version)?;
        let offset_table = Self::read_offset_table(reader, dir_version, header)?;
        let items = Self::read_items(reader, dir_version, header, &offset_table)?;

        return Ok(items);
    }
}
