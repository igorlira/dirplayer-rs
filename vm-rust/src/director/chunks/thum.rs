use binary_reader::{BinaryReader, Endian};
use anyhow::Result;
use web_sys::console;

pub struct ThumChunk {
    pub raw_data: Vec<u8>,
}

impl ThumChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<ThumChunk> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big; // TODO: why, if we only read u8?

        let mut raw_data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            raw_data.push(byte);
        }

        reader.endian = original_endian;

        console::log_1(
            &format!(
                "Thum raw_data ({} bytes): {:?}",
                raw_data.len(),
                raw_data
                    .iter()
                    .map(|b| format!("{:02X}", b))
                    .collect::<Vec<String>>()
                    .join(" ")
            )
            .into(),
        );

        Ok(ThumChunk { raw_data })
    }
}
