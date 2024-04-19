use binary_reader::BinaryReader;

pub struct BitmapChunk {
  pub data: Vec<u8>,
}

impl BitmapChunk {
  pub fn read(reader: &mut BinaryReader) -> Result<BitmapChunk, String> {
    Ok(BitmapChunk {
      data: reader.data.clone(),
    })
  }
}
