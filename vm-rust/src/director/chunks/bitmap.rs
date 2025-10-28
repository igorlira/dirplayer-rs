use binary_reader::BinaryReader;

pub struct BitmapChunk {
    pub data: Vec<u8>,
    pub version: u16,
}

impl BitmapChunk {
    pub fn read(reader: &mut BinaryReader, dir_version: u16) -> Result<BitmapChunk, String> {
        Ok(BitmapChunk {
            data: reader.data.clone(),
            version: dir_version,
        })
    }
}
