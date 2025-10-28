use binary_reader::{BinaryReader, Endian};
use web_sys::console;

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

        console::log_1(
            &format!(
                "FXmp raw_data ({} bytes): {:?}",
                raw_data.len(),
                raw_data
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<String>>()
                    .join(" ")
            )
            .into(),
        );

        Ok(EffectChunk { raw_data })
    }
}
