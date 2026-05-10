use binary_reader::BinaryReader;
use itertools::Itertools;

use crate::director::{chunks::literal::{LiteralStore, LiteralType}, lingo::datum::Datum};

use super::handler::{HandlerDef, HandlerRecord};
use crate::director::static_datum::StaticDatum;
use std::collections::{hash_map::Entry, HashMap};

#[derive(Clone)]
pub struct ScriptChunk {
    pub script_number: u16,
    pub literals: Vec<Datum>,
    pub handlers: Vec<HandlerDef>,
    pub property_name_ids: Vec<u16>,
    pub property_defaults: HashMap<u16, StaticDatum>,
}

impl ScriptChunk {
    #[allow(unused_variables)]
    pub fn from_reader(
        reader: &mut BinaryReader,
        dir_version: u16,
        capital_x: bool,
    ) -> Result<ScriptChunk, String> {
        // Lingo scripts are always big endian regardless of file endianness
        reader.set_endian(binary_reader::Endian::Big);

        reader.jmp(8);

        let /*  8 */ total_length = reader.read_u32().unwrap();
        let /* 12 */ total_length2 = reader.read_u32().unwrap();
        let /* 16 */ header_length = reader.read_u16().unwrap();
        let /* 18 */ script_number = reader.read_u16().unwrap();
        let /* 20 */ unk20 = reader.read_u16().unwrap();
        let /* 22 */ parent_number = reader.read_u16().unwrap();

        reader.jmp(38);
        let /* 38 */ script_flags = reader.read_u32().unwrap();
        let /* 42 */ unk42 = reader.read_u16().unwrap();
        let /* 44 */ cast_id = reader.read_u32().unwrap();
        let /* 48 */ factory_name_id = reader.read_u16().unwrap();
        let /* 50 */ handler_vectors_count = reader.read_u16().unwrap();
        let /* 52 */ handler_vectors_offset = reader.read_u32().unwrap();
        let /* 56 */ handler_vectors_size = reader.read_u32().unwrap();
        let /* 60 */ properties_count = reader.read_u16().unwrap() as usize;
        let /* 62 */ properties_offset = reader.read_u32().unwrap() as usize;
        let /* 66 */ globals_count = reader.read_u16().unwrap() as usize;
        let /* 68 */ globals_offset = reader.read_u32().unwrap() as usize;
        let /* 72 */ handlers_count = reader.read_u16().unwrap();
        let /* 74 */ handlers_offset = reader.read_u32().unwrap() as usize;
        let /* 78 */ literals_count = reader.read_u16().unwrap();
        let /* 80 */ literals_offset = reader.read_u32().unwrap() as usize;
        let /* 84 */ literals_data_count = reader.read_u32().unwrap();
        let /* 88 */ literals_data_offset = reader.read_u32().unwrap() as usize;

        let property_name_ids = read_varnames_table(reader, properties_count, properties_offset);
        let global_name_ids = read_varnames_table(reader, globals_count, globals_offset);

        // TODO
        // if ((scriptFlags & ScriptFlags.kScriptFlagEventScript != 0) && handlersCount > 0) {
        //   handlers[0].isGenericEvent = true;
        // }

        reader.jmp(handlers_offset);
        let handler_records = (0..handlers_count)
            .map(|_| HandlerRecord::read_record(reader, dir_version, capital_x).unwrap())
            .collect_vec();

        let handlers = handler_records
            .iter()
            .map(|record| HandlerRecord::read_data(reader, record).unwrap())
            .collect_vec();

        reader.jmp(literals_offset);
        let literal_records = (0..literals_count)
            .map(|_| LiteralStore::read_record(reader, dir_version).unwrap())
            .collect_vec();

        let has_javascript = literal_records
            .iter()
            .any(|r| matches!(r.literal_type, LiteralType::JavaScript));

        let literals = if has_javascript {
            parse_javascript_literals(reader, literals_data_offset, literals_count as usize)
        } else {
            literal_records
                .iter()
                .map(|record| LiteralStore::read_data(reader, record, literals_data_offset).unwrap())
                .collect_vec()
        };

        // === Map property IDs to real parameter names ===
        let mut property_defaults = HashMap::new();
        for (i, prop_id) in property_name_ids.iter().enumerate() {
            if let Some(literal) = literals.get(i) {
                // Property has a default value from the literal
                if let Entry::Vacant(entry) = property_defaults.entry(*prop_id) {
                    entry.insert(StaticDatum::from(literal));
                }
            }
            // Properties without literals will be initialized to Void in ScriptInstance::new()
        }

        Ok(ScriptChunk {
            script_number,
            literals,
            handlers,
            property_name_ids,
            property_defaults,
        })
    }
}

/// JavaScript Lscr chunks store compiled JS data in the literal data area
/// using a different format from standard Lingo literals. The data area contains
/// a big-endian u32 total size followed by one or more data blocks, each
/// beginning with the little-endian magic marker 0xDEAD0003.
///
/// The literal records for JS scripts are placeholders (type 0 with offset 0)
/// except for the first one (type 11 / JavaScript). The actual constant data
/// is extracted by splitting the data area at each magic marker.
fn parse_javascript_literals(
    reader: &mut BinaryReader,
    literals_data_offset: usize,
    literals_count: usize,
) -> Vec<Datum> {
    const JS_BLOCK_MAGIC: [u8; 4] = [0x03, 0x00, 0xAD, 0xDE]; // 0xDEAD0003 LE

    reader.jmp(literals_data_offset);
    let total_size = reader.read_u32().unwrap() as usize;
    let data = reader.read_bytes(total_size).unwrap();

    // Find block boundaries by scanning for the magic marker
    let mut block_offsets: Vec<usize> = Vec::new();
    for i in 0..data.len().saturating_sub(3) {
        if data[i..i + 4] == JS_BLOCK_MAGIC {
            block_offsets.push(i);
        }
    }

    // Split data into blocks at each marker
    let mut blocks: Vec<Vec<u8>> = Vec::new();
    for (idx, &start) in block_offsets.iter().enumerate() {
        let end = if idx + 1 < block_offsets.len() {
            block_offsets[idx + 1]
        } else {
            data.len()
        };
        blocks.push(data[start..end].to_vec());
    }

    // Build the literals array to match the expected count.
    // Index 0 is Void (the type-11 header record), indices 1..N hold the JS blocks.
    let mut literals = Vec::with_capacity(literals_count);
    literals.push(Datum::Void);
    for block in blocks {
        literals.push(Datum::JavaScript(block));
    }
    // Pad with Void if needed to match literals_count
    while literals.len() < literals_count {
        literals.push(Datum::Void);
    }

    literals
}

fn read_varnames_table(reader: &mut BinaryReader, count: usize, offset: usize) -> Vec<u16> {
    reader.jmp(offset);
    return (0..count).map(|_| reader.read_u16().unwrap()).collect();
}
