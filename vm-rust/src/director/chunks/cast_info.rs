use binary_reader::{BinaryReader, Endian};
use web_sys::console;

pub struct CastInfoChunk {
    pub raw_data: Vec<u8>,
}

impl CastInfoChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<CastInfoChunk, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let mut raw_data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            raw_data.push(byte);
        }

        reader.endian = original_endian;

        console::log_1(
            &format!(
                "Cinf raw_data ({} bytes): {:?}",
                raw_data.len(),
                raw_data
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<String>>()
                    .join(" ")
            )
            .into(),
        );

        Ok(CastInfoChunk { raw_data })
    }
}
