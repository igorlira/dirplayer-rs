use binary_reader::BinaryReader;

use crate::utils::log_i;

pub struct KeyTableEntry {
    pub section_id: u32, // TODO i32?
    pub cast_id: u32,    // TODO i32?
    pub fourcc: u32,
}

impl KeyTableEntry {
    pub fn from_reader(
        reader: &mut BinaryReader,
        _dir_version: u16,
    ) -> Result<KeyTableEntry, String> {
        return Ok(KeyTableEntry {
            section_id: reader.read_u32().unwrap(),
            cast_id: reader.read_u32().unwrap(),
            fourcc: reader.read_u32().unwrap(),
        });
    }
}

pub struct KeyTableChunk {
    pub entry_size: u16, // Should always be 12 (3 uint32's)
    pub entry_size2: u16,
    pub entry_count: u32,
    pub used_count: u32,
    pub entries: Vec<KeyTableEntry>,
}

impl KeyTableChunk {
    pub fn from_reader(
        reader: &mut BinaryReader,
        dir_version: u16,
    ) -> Result<KeyTableChunk, String> {
        let entry_size = reader.read_u16().unwrap();
        let entry_size2 = reader.read_u16().unwrap();
        let entry_count = reader.read_u32().unwrap();
        let used_count = reader.read_u32().unwrap();

        return Ok(KeyTableChunk {
            entry_size: entry_size,
            entry_size2: entry_size2,
            entry_count: entry_count,
            used_count: used_count,
            entries: (0..entry_count)
                .map(|_| KeyTableEntry::from_reader(reader, dir_version).unwrap())
                .collect(),
        });
    }
}
