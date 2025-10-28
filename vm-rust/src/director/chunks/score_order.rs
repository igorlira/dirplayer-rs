use binary_reader::{BinaryReader, Endian};

use web_sys::console;

pub struct SordChunk {
    pub raw_data: Vec<u8>,
}

impl SordChunk {
    pub fn from_reader(reader: &mut BinaryReader) -> Result<SordChunk, String> {
        let original_endian = reader.endian;
        reader.endian = Endian::Big;

        // Read all bytes until EOF
        let mut raw_data = Vec::new();
        while let Ok(byte) = reader.read_u8() {
            raw_data.push(byte);
        }

        reader.endian = original_endian;

        console::log_1(&format!("Read {} bytes for Sord chunk", raw_data.len()).into());

        if raw_data.len() < 20 {
            return Err("Sord chunk too small to contain header".into());
        }

        let header = &raw_data[..20];

        let marker = u32::from_be_bytes([header[0], header[1], header[2], header[3]]);

        let channels = u16::from_be_bytes([header[0], header[1]]);
        let bits_per_sample = u16::from_be_bytes([header[2], header[3]]);
        let sample_rate = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
        let data_offset = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);
        let data_length = u32::from_be_bytes([header[12], header[13], header[14], header[15]]);
        let codec = u16::from_be_bytes([header[16], header[17]]);
        let flags = u16::from_be_bytes([header[18], header[19]]);

        web_sys::console::log_1(&format!(
            "Parsed Sord header:\n marker={} channels={} bits={} sample_rate={} offset={} length={} codec={} flags={}",
            marker, channels, bits_per_sample, sample_rate, data_offset, data_length, codec, flags
        ).into());

        // Optional: save remaining bytes for later
        let extra_data = &raw_data[20..];
        web_sys::console::log_1(
            &format!("Extra {} bytes remaining in Sord chunk", extra_data.len()).into(),
        );

        Ok(SordChunk { raw_data })
    }
}
