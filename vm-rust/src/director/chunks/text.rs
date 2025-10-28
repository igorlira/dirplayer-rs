use binary_reader::BinaryReader;

use crate::io::reader::DirectorExt;

pub struct TextChunk {
    pub offset: usize,
    pub text_length: usize,
    pub data_length: usize,
    pub text: String,
    pub data: Vec<u8>,
}

impl TextChunk {
    pub fn read(reader: &mut BinaryReader) -> Result<TextChunk, String> {
        reader.set_endian(binary_reader::Endian::Big);

        let offset = reader.read_u32().unwrap() as usize;
        if offset != 12 {
            return Err("Stxt init: unhandled offset".to_owned());
        }

        let text_length = reader.read_u32().unwrap() as usize;
        let data_length = reader.read_u32().unwrap() as usize;

        Ok(TextChunk {
            offset,
            text_length,
            data_length,
            text: reader.read_string(text_length).unwrap(),
            data: reader.read_bytes(data_length).unwrap().to_vec(),
        })
    }
}
