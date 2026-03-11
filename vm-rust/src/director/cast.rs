use std::collections::HashMap;

use binary_reader::BinaryReader;
use itertools::Itertools;
use log::{debug, error};

use crate::{
    director::{file::get_children_of_chunk, utils::fourcc_to_string},
    utils::log_i,
};

use super::{
    chunks::{
        cast_member::CastMemberDef, key_table::KeyTableChunk, script::ScriptChunk, ChunkContainer,
    },
    file::{
        get_cast_member_chunk, get_chunk, get_script_chunk, get_script_context_chunk,
        get_script_context_key_entry_for_cast, get_script_names_chunk,
    },
    lingo::script::ScriptContext,
    rifx::RIFXReaderContext,
    utils::FOURCC,
};

pub struct CastDef {
    pub id: u32,
    pub name: String,
    pub members: HashMap<u32, CastMemberDef>,
    pub lctx: Option<ScriptContext>,
    pub capital_x: bool,
    pub dir_version: u16,
    /// Maps section_id -> (member_number, member_name) for all chunks belonging to members
    pub section_to_member: HashMap<u32, (u32, String)>,
    /// The section_id of the Lctx/LctX chunk for this cast (if any)
    pub lctx_section_id: Option<u32>,
    /// Section IDs of chunks referenced internally by Lctx (Lscr scripts + Lnam names).
    /// These are NOT in the KeyTable as children of Lctx, so they must be tracked separately.
    pub lctx_child_section_ids: Vec<u32>,
    /// Offset to adjust bitmap clutId values from Config-based numbering to MCsL-based numbering.
    /// Computed as `config.min_member - mcsl.min_member`. Applied to positive palette_id values
    /// in bitmap member data to convert from file-stored references to loaded member numbers.
    pub palette_id_offset: i16,
}

impl CastDef {
    pub fn from(
        name: String,
        id: u32,
        min_member: u16,
        member_ids: Vec<u32>,
        reader: &mut BinaryReader,
        chunk_container: &mut ChunkContainer,
        rifx: &mut RIFXReaderContext,
        key_table: &KeyTableChunk,
        palette_id_offset: i16,
    ) -> Result<CastDef, anyhow::Error> {
        // TODO script names, scripts
        let lctx_entry =
            get_script_context_key_entry_for_cast(reader, chunk_container, key_table, rifx, id);
        let lctx = lctx_entry.and_then(|entry| {
            get_script_context_chunk(
                reader,
                chunk_container,
                rifx,
                entry.fourcc,
                entry.section_id,
            )
        });
        let script_names = lctx.as_ref().and_then(|lctx| {
            get_script_names_chunk(
                reader,
                chunk_container,
                rifx,
                FOURCC("Lnam"),
                lctx.lnam_section_id,
            )
        });
        let capital_x = lctx_entry.is_some_and(|e| e.fourcc == FOURCC("LctX"));
        let lctx_section_id = lctx_entry.map(|e| e.section_id);

        let mut members: HashMap<u32, CastMemberDef> = HashMap::new();
        let mut section_to_member: HashMap<u32, (u32, String)> = HashMap::new();
        for i in 0..member_ids.len() {
            let section_id = member_ids[i];
            if section_id <= 0 {
                continue;
            }
            let member_id = i as u16 + min_member;
            let member = get_cast_member_chunk(reader, chunk_container, rifx, section_id);
            let children_entries = get_children_of_chunk(&section_id, key_table);

            let member_name = member.member_info.as_ref()
                .map(|info| info.name.clone())
                .unwrap_or_default();

            // Map the CASt chunk itself and all its children to this member
            section_to_member.insert(section_id, (member_id as u32, member_name.clone()));
            for child_entry in &children_entries {
                section_to_member.insert(child_entry.section_id, (member_id as u32, member_name.clone()));
            }

            let children = children_entries
                .iter()
                .map(|x| {
                    let fourcc_str: String = fourcc_to_string(x.fourcc);
                    let child = get_chunk(reader, chunk_container, rifx, x.fourcc, x.section_id);
                    if let Err(err) = &child {
                        error!(
                            "Failed to read child chunk type='{}' section_id={} for member {}: {}",
                            fourcc_str, x.section_id, member_id, err
                        );
                    }
                    child.ok()
                })
                .collect_vec();

            // log_i(format_args!("Member {member_id} name: \"{}\" chunk: {section_id} children: {}", member.member_info.name, children.len()).to_string().as_str());
            let member_def = CastMemberDef {
                chunk: member,
                children,
            };

            members.insert(member_id as u32, member_def);
        }

        let mut scripts: HashMap<u32, ScriptChunk> = HashMap::new();
        let mut lctx_child_section_ids: Vec<u32> = Vec::new();
        if let Some(lctx) = &lctx {
            // Collect Lnam section_id
            if lctx.lnam_section_id > 0 {
                lctx_child_section_ids.push(lctx.lnam_section_id);
            }
            for i in 0..lctx.entry_count {
                let section = &lctx.section_map[i as usize];
                if section.section_id > -1 {
                    // Collect Lscr section_id
                    lctx_child_section_ids.push(section.section_id as u32);
                    let script = get_script_chunk(
                        reader,
                        chunk_container,
                        rifx,
                        FOURCC("Lscr"),
                        section.section_id as u32,
                    );
                    // TODO script.setContext(this);
                    if let Some(script) = script {
                        scripts.insert(i + 1, script);
                    }
                }
            }
        }
        return Ok(CastDef {
            id,
            name: name,
            members: members,
            lctx: lctx.map(|_| ScriptContext {
                scripts,
                names: script_names.map_or(Vec::new(), |x| x.names),
            }),
            capital_x,
            dir_version: rifx.dir_version,
            section_to_member,
            lctx_section_id,
            lctx_child_section_ids,
            palette_id_offset,
        });
    }
}
