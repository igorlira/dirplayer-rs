use binary_reader::BinaryReader;
use itertools::Itertools;

use crate::director::{chunks::literal::LiteralStore, lingo::datum::Datum};

use super::handler::{HandlerDef, HandlerRecord};
use crate::director::static_datum::StaticDatum;
use std::collections::{hash_map::Entry, HashMap};

#[derive(Clone)]
pub struct ScriptChunk {
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

        let literals = literal_records
            .iter()
            .map(|record| LiteralStore::read_data(reader, record, literals_data_offset).unwrap())
            .collect_vec();

        // === Map property IDs to real parameter names ===
        let mut property_defaults = HashMap::new();
        for (i, prop_id) in property_name_ids.iter().enumerate() {
            if let Some(literal) = literals.get(i) {
                // Property has a default value from the literal
                if let Entry::Vacant(entry) = property_defaults.entry(*prop_id) {
                    entry.insert(StaticDatum::from(literal.clone()));
                }
            }
            // Properties without literals will be initialized to Void in ScriptInstance::new()
        }

        Ok(ScriptChunk {
            literals,
            handlers,
            property_name_ids,
            property_defaults,
        })
    }
}

fn read_varnames_table(reader: &mut BinaryReader, count: usize, offset: usize) -> Vec<u16> {
    reader.jmp(offset);
    return (0..count).map(|_| reader.read_u16().unwrap()).collect();
}
