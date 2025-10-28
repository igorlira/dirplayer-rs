use binary_reader::{BinaryReader, Endian};

#[allow(dead_code)]
pub struct ScriptContextChunk {
    pub entry_count: u32,
    entry_count2: u32,
    entries_offset: usize,
    pub lnam_section_id: u32,
    valid_count: u16,
    flags: u16,
    free_pointer: u16,
    pub section_map: Vec<ScriptContextMapEntry>,
}

#[allow(dead_code)]
pub struct ScriptContextMapEntry {
    unknown0: u32,
    pub section_id: i32,
    unknown1: u16,
    unknown2: u16,
}

impl ScriptContextMapEntry {
    #[allow(unused_variables)]
    pub fn from_reader(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<ScriptContextMapEntry, String> {
        return Ok(ScriptContextMapEntry {
            unknown0: reader.read_u32().unwrap(),
            section_id: reader.read_i32().unwrap(),
            unknown1: reader.read_u16().unwrap(),
            unknown2: reader.read_u16().unwrap(),
        });
    }
}

impl ScriptContextChunk {
    #[allow(unused_variables)]
    pub fn from_reader(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<ScriptContextChunk, String> {
        reader.set_endian(Endian::Big);

        let unknown0 = reader.read_u32().unwrap();
        let unknown1 = reader.read_u32().unwrap();
        let entry_count = reader.read_u32().unwrap();
        let entry_count2 = reader.read_u32().unwrap();
        let entries_offset = reader.read_u16().unwrap() as usize;
        let unknown2 = reader.read_u16().unwrap();
        let unknown3 = reader.read_u32().unwrap();
        let unknown4 = reader.read_u32().unwrap();
        let unknown5 = reader.read_u32().unwrap();
        let lnam_section_id = reader.read_u32().unwrap();
        let valid_count = reader.read_u16().unwrap();
        let flags = reader.read_u16().unwrap();
        let free_pointer = reader.read_u16().unwrap();

        reader.jmp(entries_offset);
        let section_map = (0..entry_count)
            .map(|_| ScriptContextMapEntry::from_reader(reader, dir_version).unwrap())
            .collect();

        return Ok(ScriptContextChunk {
            entry_count,
            entry_count2,
            entries_offset,
            lnam_section_id,
            valid_count,
            flags,
            free_pointer,
            section_map,
        });
    }
}
