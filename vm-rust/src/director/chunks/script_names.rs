use std::io::Error;
use binary_reader::BinaryReader;

use crate::io::reader::DirectorExt;

pub struct ScriptNamesChunk {
    pub names: Vec<String>,
}

impl ScriptNamesChunk {
    #[allow(unused_variables)]
    pub fn from_reader(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<ScriptNamesChunk, Error> {
        reader.set_endian(binary_reader::Endian::Big);

        let unknown0 = reader.read_u32()?;
        let unknown1 = reader.read_u32()?;
        let len1 = reader.read_u32()?;
        let len2 = reader.read_u32()?;
        let names_offset = reader.read_u16()? as usize;
        let names_count = reader.read_u16()?;

        reader.jmp(names_offset);
        let names = (0..names_count)
            .map(|_| reader.read_pascal_string())
            .collect::<Result<_, _>>()?;

        return Ok(ScriptNamesChunk { names });
    }
}
