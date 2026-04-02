use binary_reader::BinaryReader;

#[derive(Debug, Clone)]
pub struct CuePointsChunk {
    pub names: Vec<String>,
    pub times: Vec<u32>,
}

impl CuePointsChunk {
    /// Parse a cupt chunk.
    /// Format: u32 BE count, then per entry: u32 BE time_ms + char[32] name (null-terminated).
    pub fn from_reader(reader: &mut BinaryReader) -> Result<Self, String> {
        let original_endian = reader.endian;
        reader.endian = binary_reader::Endian::Big;

        let count = reader.read_u32().map_err(|e| e.to_string())? as usize;

        let mut names = Vec::with_capacity(count);
        let mut times = Vec::with_capacity(count);

        for _ in 0..count {
            let time = reader.read_u32().map_err(|e| e.to_string())?;
            times.push(time);

            let name_bytes = reader.read_bytes(32).map_err(|e| e.to_string())?;
            // Null-terminate and convert to string
            let null_pos = name_bytes.iter().position(|&b| b == 0).unwrap_or(32);
            let name = String::from_utf8_lossy(&name_bytes[..null_pos]).to_string();
            names.push(name);
        }

        reader.endian = original_endian;

        Ok(CuePointsChunk { names, times })
    }
}
