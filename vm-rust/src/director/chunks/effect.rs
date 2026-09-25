use binary_reader::{BinaryReader, Endian};
use log::debug;

pub struct EffectChunk {
    pub raw_data: Vec<u8>,
}

impl EffectChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<EffectChunk, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let mut raw_data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            raw_data.push(byte);
        }

        reader.endian = original_endian;

        debug!(
            "FXmp raw_data ({} bytes): {:?}",
            raw_data.len(),
            crate::director::chunks::hex_preview(&raw_data)
        );

        Ok(EffectChunk { raw_data })
    }
}
