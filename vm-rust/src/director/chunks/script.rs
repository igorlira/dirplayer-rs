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

/// JavaScript Lscr chunks store ONE compiled JS script in the literal data area.
/// The on-wire format is Mozilla SpiderMonkey 1.5's XDR-serialized JSScript
/// (magic `0xDEAD0003`), produced by `js_XDRScript` in jsdmx/src/jsscript.c.
///
/// Top-level structure of the literal-data region:
///   u32 BE  total_size       (Director framing, payload byte count)
///   bytes[total_size]        XDR-serialized JSScript
///
/// The XDR stream starts with `0xDEAD0003 length prologLength version bytecode...`.
/// Inner function bodies are themselves XDR'd JSScripts and re-use the same
/// magic — earlier versions of this parser mistakenly split on every magic
/// occurrence, producing a flat list of "blocks" that desynced the atom
/// table. The actual decoding lives in `player::js_lingo::decode_script`,
/// which descends function objects recursively.
///
/// We store the entire payload as a single `Datum::JavaScript` at literal
/// index 0 (which is the type-11 record). The other slots stay `Void` to
/// match the placeholder records — keeping `literals.len() == literals_count`
/// so downstream indexing doesn't have to special-case JS scripts.
fn parse_javascript_literals(
    reader: &mut BinaryReader,
    literals_data_offset: usize,
    literals_count: usize,
) -> Vec<Datum> {
    reader.jmp(literals_data_offset);
    let total_size = reader.read_u32().unwrap() as usize;
    let data = reader.read_bytes(total_size).unwrap();

    let mut literals = Vec::with_capacity(literals_count);
    literals.push(Datum::JavaScript(data.to_vec()));
    while literals.len() < literals_count {
        literals.push(Datum::Void);
    }
    literals
}

fn read_varnames_table(reader: &mut BinaryReader, count: usize, offset: usize) -> Vec<u16> {
    reader.jmp(offset);
    return (0..count).map(|_| reader.read_u16().unwrap()).collect();
}
