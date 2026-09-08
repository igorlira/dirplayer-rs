use binary_reader::BinaryReader;

pub struct BitmapChunk {
    pub data: Vec<u8>,
    pub version: u16,
}

impl BitmapChunk {
    /// Build from bytes the caller already owns. `read` below clones instead,
    /// which is a full copy of every bitmap in the movie; `make_chunk_in` hands
    /// the buffer over when it may.
    pub fn from_data(data: Vec<u8>, dir_version: u16) -> BitmapChunk {
        BitmapChunk {
            data,
            version: dir_version,
        }
    }

    pub fn read(reader: &mut BinaryReader, dir_version: u16) -> Result<BitmapChunk, String> {
        Ok(BitmapChunk::from_data(reader.data.clone(), dir_version))
    }
}
