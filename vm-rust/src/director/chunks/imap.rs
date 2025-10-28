use binary_reader::BinaryReader;

#[allow(dead_code)]
pub struct InitialMapChunk {
    version: u32,
    mmap_offset: usize,
    director_version: u32,
    unused1: u32,
    unused2: u32,
    unused3: u32,
}

impl InitialMapChunk {
    pub fn from_reader(_: &mut BinaryReader, _: u16) -> Result<InitialMapChunk, String> {
        return Err("TODO".to_owned());
    }
}
