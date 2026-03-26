use binary_reader::{BinaryReader, Endian};
pub struct ThumChunk {
    pub raw_data: Vec<u8>,
}

impl ThumChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<ThumChunk, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        let mut raw_data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            raw_data.push(byte);
        }

        reader.endian = original_endian;

        let dump = format!(
            "Thum raw_data ({} bytes): {:?}",
            raw_data.len(),
            raw_data
                .iter()
                .map(|b| format!("{:02X}", b))
                .collect::<Vec<String>>()
                .join(" ")
        );
        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(&dump.as_str().into());
        #[cfg(not(target_arch = "wasm32"))]
        println!("{}", dump);

        Ok(ThumChunk { raw_data })
    }
}
