use binary_reader::BinaryReader;

pub struct PaletteChunk {
    pub colors: Vec<(u8, u8, u8)>,
}

impl PaletteChunk {
    pub fn from_reader(reader: &mut BinaryReader, _: u16) -> Result<PaletteChunk, String> {
        reader.set_endian(binary_reader::Endian::Big);

        let mut colors = Vec::new();
        for _ in 0..256 {
            let r = reader.read_u8().unwrap_or(255);
            reader.read_u8().ok();
            let g = reader.read_u8().unwrap_or(0);
            reader.read_u8().ok();
            let b = reader.read_u8().unwrap_or(255);
            reader.read_u8().ok();
            colors.push((r, g, b));
        }

        return Ok(PaletteChunk { colors });
    }
}
