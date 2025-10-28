use std::collections::HashMap;
use std::str::FromStr;

use crate::console_warn;
use crate::director::chunks::config::ConfigChunk;
use crate::director::chunks::key_table::KeyTableChunk;
use crate::director::guid::*;
use crate::director::rifx::RIFXReaderContext;
use crate::director::utils::*;
use crate::io::reader::DirectorExt;
use crate::utils::log_i;
use binary_reader::BinaryReader;
use itertools::Itertools;
use log::warn;
use url::Url;

use super::cast::CastDef;
use super::chunks::cast::CastChunk;
use super::chunks::cast_info::CastInfoChunk;
use super::chunks::cast_list::CastListChunk;
use super::chunks::cast_list::CastListEntry;
use super::chunks::cast_member::CastMemberChunk;
use super::chunks::effect::EffectChunk;
use super::chunks::key_table::KeyTableEntry;
use super::chunks::lctx::ScriptContextChunk;
use super::chunks::make_chunk;
use super::chunks::media::MediaChunk;
use super::chunks::score::FrameLabelsChunk;
use super::chunks::score::ScoreChunk;
use super::chunks::score_order::SordChunk;
use super::chunks::script::ScriptChunk;
use super::chunks::script_names::ScriptNamesChunk;
use super::chunks::sound::SoundChunk;
use super::chunks::thum::ThumChunk;
use super::chunks::xmedia::XMediaChunk;
use super::chunks::Chunk;
use super::chunks::ChunkContainer;
use super::chunks::ChunkInfo;
use binary_reader::Endian;

pub struct DirectorFile {
    pub base_path: Url,
    pub file_name: String,
    //pub endian: Endian,
    pub version: u16,
    pub cast_entries: Vec<CastListEntry>,
    pub casts: Vec<CastDef>,
    pub config: ConfigChunk,
    pub score: Option<ScoreChunk>,
    pub frame_labels: Option<FrameLabelsChunk>,
    pub score_order: Option<SordChunk>,
    pub media: Option<MediaChunk>,
    pub xmedia: Option<XMediaChunk>,
    pub cast_info: Option<CastInfoChunk>,
    pub effect: Option<EffectChunk>,
    pub thum: Option<ThumChunk>,
    pub chunk_container: ChunkContainer,
}

// macro_rules! console_log {
//   // Note that this is using the `log` function imported above during
//   // `bare_bones`
//   ($($t:tt)*) => (web_sys::console::log_1(&JsValue::from_str(format_args!($($t)*).to_string().as_str())))
// }

impl DirectorFile {
    #[allow(unused_variables, unused_assignments)]
    fn read(
        file_name: String,
        base_path: Url,
        reader: &mut BinaryReader,
    ) -> Result<DirectorFile, String> {
        reader.set_endian(binary_reader::Endian::Big);

        let mut chunk_container = ChunkContainer {
            cached_chunk_views: HashMap::new(),
            chunk_info: HashMap::new(),
            deserialized_chunks: HashMap::new(),
        };

        let meta_fourcc = reader.read_u32().unwrap();
        if meta_fourcc == FOURCC("XFIR") {
            reader.set_endian(binary_reader::Endian::Little);
        }
        //self.endian = reader.endian;

        let _ = reader.read_u32().unwrap(); // meta length
        let codec = reader.read_u32().unwrap();
        let mut after_burned = false;
        let mut ils_body_offset: usize = 0;

        if codec == FOURCC("MV93") || codec == FOURCC("MC95") {
            // read memory map
            return Err("read mmap not implemented".to_owned());
        } else if codec == FOURCC("FGDM") || codec == FOURCC("FGDC") {
            after_burned = true;
            ils_body_offset = read_after_burner_map(
                reader,
                &mut chunk_container.cached_chunk_views,
                &mut chunk_container.chunk_info,
            )
            .unwrap();
        } else {
            return Err("Invalid codec".to_owned());
        }

        let mut rifx = RIFXReaderContext {
            after_burned: after_burned,
            ils_body_offset: ils_body_offset,
            dir_version: 0,
            lctx_capital_x: false,
        };

        let key_table = read_key_table(reader, &mut chunk_container, &mut rifx).unwrap();

        let config = read_config(reader, &mut chunk_container, &mut rifx).unwrap();

        rifx.dir_version = human_version(config.director_version);
        let dot_syntax = rifx.dir_version >= 700;

        // info!("width={}, height={}", config.movie_right - config.movie_left, config.movie_bottom - config.movie_top);

        let (cast_entries, casts) =
            read_casts(reader, &mut chunk_container, &mut rifx, &key_table, &config).unwrap();

        // for cast in &casts {
        //   info!("Cast {} members ({})", cast.name, cast.members.len());
        //   for (id, member) in &cast.members {
        //     info!("- id: {} {}", id, member.chunk.member_info.name)
        //   }
        // }

        let score = get_score_chunk(reader, &mut chunk_container, &mut rifx);

        let frame_labels = get_frame_labels_chunk(reader, &mut chunk_container, &mut rifx);

        let score_order = get_score_order_chunk(reader, &mut chunk_container, &mut rifx);

        let media = get_media_chunk(reader, &mut chunk_container, &mut rifx);

        let xmedia = get_xmedia_chunk(reader, &mut chunk_container, &mut rifx);

        let cast_info = get_cast_info_chunk(reader, &mut chunk_container, &mut rifx);

        let effect = get_effect_chunk(reader, &mut chunk_container, &mut rifx);

        let thum = get_thum_chunk(reader, &mut chunk_container, &mut rifx);

        return Ok(DirectorFile {
            base_path,
            file_name,
            version: rifx.dir_version,
            casts,
            cast_entries,
            config,
            score,
            frame_labels,
            score_order,
            media,
            xmedia,
            cast_info,
            effect,
            thum,
            chunk_container,
        });
    }
}

pub fn get_variable_multiplier(capital_x: bool, dir_version: u16) -> u32 {
    // TODO: Determine what version this changed to 1.
    // For now approximating it with the point at which Lctx changed to LctX.
    if capital_x {
        return 1;
    }
    if dir_version >= 500 {
        return 8;
    }
    return 6;
}

fn read_casts(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    key_table: &KeyTableChunk,
    config: &ConfigChunk,
) -> Result<(Vec<CastListEntry>, Vec<CastDef>), String> {
    let mut internal = true;
    let mut casts: Vec<CastDef> = Vec::new();

    if rifx.dir_version >= 500 {
        let cast_list = get_cast_list_chunk(reader, chunk_container, rifx);
        if cast_list.is_some() {
            let cast_list = cast_list.unwrap();
            for cast_entry in &cast_list.entries {
                // info!("Cast: {} id: {}", &cast_entry.name, &cast_entry.id);
                let cast = get_cast_chunk_for_cast(
                    reader,
                    chunk_container,
                    rifx,
                    key_table,
                    &cast_entry.id,
                );
                if let Some(cast) = cast {
                    // TODO cast.populate(castEntry.name, castEntry.id, castEntry.minMember);
                    // info!("Cast {} member count: {}", cast_entry.name, cast.member_ids.len());
                    casts.push(
                        CastDef::from(
                            cast_entry.name.to_owned(),
                            cast_entry.id,
                            cast_entry.min_member,
                            cast.member_ids.to_vec(),
                            reader,
                            chunk_container,
                            rifx,
                            key_table,
                        )
                        .unwrap(),
                    );
                    // TODO populate
                }
            }

            return Ok((cast_list.entries, casts));
        } else {
            internal = false;
        }
    }

    let cast = get_first_chunk(reader, chunk_container, rifx, FOURCC("CAS*"));
    if let Some(Chunk::Cast(cast)) = cast {
        // TODO
        //cast.populate(internal ? "Internal" : "External", 1024, config!.minMember);
        casts.push(
            CastDef::from(
                (if internal { "Internal" } else { "External" }).to_string(),
                1024,
                config.min_member,
                cast.member_ids.to_vec(),
                reader,
                chunk_container,
                rifx,
                key_table,
            )
            .unwrap(),
        );
        // TODO populate

        return Ok((Vec::new(), casts));
    }

    log_i("No cast!");
    return Ok((Vec::new(), casts));
}

fn find_key_table_entry_for_cast<'b>(
    key_table: &'b KeyTableChunk,
    cast_id: &u32,
) -> Option<&'b KeyTableEntry> {
    let res = key_table
        .entries
        .iter()
        .find(|entry| entry.cast_id == *cast_id && entry.fourcc == FOURCC("CAS*"));
    return res;
}

fn get_cast_chunk_for_cast(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    key_table: &KeyTableChunk,
    cast_id: &u32,
) -> Option<CastChunk> {
    let key_entry = find_key_table_entry_for_cast(key_table, cast_id);
    if let Some(key_entry) = key_entry {
        return Some(get_cast_chunk(
            reader,
            chunk_container,
            rifx,
            key_entry.section_id,
        ));
    } else {
        return None;
    }
}

pub fn get_cast_member_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    section_id: u32,
) -> CastMemberChunk {
    let chunk = get_chunk(reader, chunk_container, rifx, FOURCC("CASt"), section_id).unwrap();
    if let Chunk::CastMember(member_chunk) = chunk {
        return member_chunk;
    } else {
        panic!("Not a cast member chunk");
    }
}

pub fn get_cast_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    section_id: u32,
) -> CastChunk {
    let chunk = get_chunk(reader, chunk_container, rifx, FOURCC("CAS*"), section_id).unwrap();
    if let Chunk::Cast(cast_chunk) = chunk {
        return cast_chunk;
    } else {
        panic!("Not a cast chunk");
    }
}

pub fn get_cast_list_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<CastListChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("MCsL"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::CastList(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a cast list chunk");
    }
}

pub fn get_score_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<ScoreChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("VWSC"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::Score(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a score chunk");
    }
}

pub fn get_media_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<MediaChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("ediM"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::Media(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a media chunk");
    }
}

pub fn get_xmedia_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<XMediaChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("XMED"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::XMedia(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a xmedia chunk");
    }
}

pub fn get_thum_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<ThumChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("Thum"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::Thum(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a Thum chunk");
    }
}

pub fn get_cast_info_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<CastInfoChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("Cinf"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::CstInfo(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a Cinf chunk");
    }
}

pub fn get_effect_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<EffectChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("FXmp"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::Effect(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a FXmp chunk");
    }
}

pub fn get_score_order_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<SordChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("Sord"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::ScoreOrder(chunk_data) = chunk.unwrap() {
        return Some(chunk_data);
    } else {
        panic!("Not a score order chunk");
    }
}

pub fn get_script_context_key_entry_for_cast<'a>(
    _reader: &mut BinaryReader,
    _chunk_container: &mut ChunkContainer,
    key_table: &'a KeyTableChunk,
    _rifx: &RIFXReaderContext,
    cast_id: u32,
) -> Option<&'a KeyTableEntry> {
    return key_table.entries.iter().find(|entry| {
        entry.cast_id == cast_id
            && (entry.fourcc == FOURCC("Lctx") || entry.fourcc == FOURCC("LctX"))
    });
}

pub fn get_script_context_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
    section_id: u32,
) -> Option<ScriptContextChunk> {
    let chunk = get_chunk(reader, chunk_container, rifx, fourcc, section_id).unwrap();

    if let Chunk::ScriptContext(context) = chunk {
        return Some(context);
    } else {
        panic!("Not a cast chunk");
    }
}

pub fn get_script_names_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
    section_id: u32,
) -> Option<ScriptNamesChunk> {
    let chunk = get_chunk(reader, chunk_container, rifx, fourcc, section_id).unwrap();

    if let Chunk::ScriptNames(names) = chunk {
        return Some(names);
    } else {
        panic!("Not a script names chunk");
    }
}

pub fn get_frame_labels_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Option<FrameLabelsChunk> {
    let chunk = get_first_chunk(reader, chunk_container, rifx, FOURCC("VWLB"));
    if chunk.is_none() {
        return None;
    } else if let Chunk::FrameLabels(labels) = chunk.unwrap() {
        return Some(labels);
    } else {
        panic!("Not a frame labels chunk");
    }
}

pub fn get_script_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
    section_id: u32,
) -> Option<ScriptChunk> {
    let chunk = get_chunk(reader, chunk_container, rifx, fourcc, section_id).unwrap();

    if let Chunk::Script(script) = chunk {
        return Some(script);
    } else {
        panic!("Not a script chunk");
    }
}

fn read_config(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Result<ConfigChunk, String> {
    let info = get_first_chunk_info(&chunk_container.chunk_info, FOURCC("DRCF")).or(
        get_first_chunk_info(&chunk_container.chunk_info, FOURCC("VWCF")),
    );

    match info {
        Some(info) => {
            if let Chunk::Config(config) =
                get_chunk(reader, chunk_container, rifx, info.fourcc, info.id).unwrap()
            {
                return Ok(config);
            } else {
                panic!("Not a config chunk");
            }
        }
        None => {
            return Err("No config chunk!".to_owned());
        }
    }
}

fn read_after_burner_map(
    reader: &mut BinaryReader,
    cached_chunk_views: &mut HashMap<u32, Vec<u8>>,
    chunk_info: &mut HashMap<u32, ChunkInfo>,
) -> Result<usize, String> {
    let start: usize;
    let end: usize;

    // File version
    if reader.read_u32().unwrap() != FOURCC("Fver") {
        return Err("readAfterburnerMap(): Fver expected but not found".to_owned());
    }

    let fver_length = reader.read_var_int().unwrap();
    start = reader.pos;
    let fver_version = reader.read_var_int().unwrap();
    // info!("Fver: version {}", fver_version);
    if fver_version >= 0x401 {
        let _imap_version = reader.read_var_int().unwrap();
        let _director_version = reader.read_var_int().unwrap();
        // info!("Fver: imapVersion: {} directorVersion: {}", imap_version, director_version);
    }
    if fver_version >= 0x501 {
        let version_string_len = reader.read_u8().unwrap();
        let _fver_version_string = String::from_utf8(
            reader
                .read_bytes(version_string_len as usize)
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        // info!("Fver: versionString: {}", fver_version_string);
    }
    end = reader.pos;

    if end - start != fver_length as usize {
        // info!("read_after_burner_map(): Expected Fver of length {} but read {} bytes", fver_length, end - start);
        reader.jmp(start + fver_length as usize);
    }

    // Compression types
    if reader.read_u32().unwrap() != FOURCC("Fcdr") {
        return Err("readAfterburnerMap(): Fcdr expected but not found".to_owned());
    }

    let fcdr_length = reader.read_var_int().unwrap();
    let fcdr_uncomp = reader.read_zlib_bytes(fcdr_length as usize).unwrap();

    let mut fcdr_reader = BinaryReader::from_vec(&fcdr_uncomp);
    fcdr_reader.set_endian(reader.endian);

    let compression_type_count = fcdr_reader.read_u16().unwrap();
    let compression_ids: Vec<MoaID> = (0..compression_type_count)
        .map(|_| MoaID::from_reader(&mut fcdr_reader))
        .collect();
    let compression_descs: Vec<String> = (0..compression_type_count)
        .map(|_| fcdr_reader.read_cstr().unwrap())
        .collect();

    // for desc in &compression_descs {
    //   info!("{}", desc);
    // }

    if fcdr_reader.pos != fcdr_reader.length {
        warn!(
            "readAfterburnerMap(): Fcdr has uncompressed length {} but read {} bytes",
            fcdr_reader.length, fcdr_reader.pos
        );
    }

    // info!("Fcdr: {} compression types", compression_type_count);

    for i in 0..compression_type_count {
        let _id = &compression_ids[i as usize];
        let _desc = &compression_descs[i as usize];
        // info!("Fcdr: type {}: {} \"{}\"", i, id, desc);
    }

    if reader.read_u32().unwrap() != FOURCC("ABMP") {
        return Err("RIFXArchive::readAfterburnerMap(): ABMP expected but not found".to_owned());
    }

    let abmp_length = reader.read_var_int().unwrap();
    let abmp_end = reader.pos + abmp_length as usize;
    let _abmp_compression_type = reader.read_var_int().unwrap();
    let abmp_uncomp_length = reader.read_var_int().unwrap();
    // info!("ABMP: length: {} compressionType: {} uncompressedLength: {}", abmp_length, abmp_compression_type, abmp_uncomp_length);

    let abmp_uncomp = reader.read_zlib_bytes(abmp_end - reader.pos).unwrap();
    if abmp_uncomp.len() != abmp_uncomp_length as usize {
        warn!(
            "ABMP: Expected uncompressed length {} but got length {}",
            abmp_uncomp_length,
            abmp_uncomp.len()
        );
    }
    let mut abmp_reader = BinaryReader::from_vec(&abmp_uncomp);
    abmp_reader.set_endian(reader.endian);

    let _abmp_unk1 = abmp_reader.read_var_int().unwrap();
    let _abmp_unk2 = abmp_reader.read_var_int().unwrap();
    let res_count = abmp_reader.read_var_int().unwrap();
    // info!("ABMP: unk1: {} unk2: {} resCount: {}", abmp_unk1, abmp_unk2, res_count);

    for _ in 0..res_count {
        let res_id = abmp_reader.read_var_int().unwrap() as u32;
        let offset = abmp_reader.read_var_int().unwrap() as usize;
        let comp_size = abmp_reader.read_var_int().unwrap() as usize;
        let uncomp_size = abmp_reader.read_var_int().unwrap() as usize;
        let compression_type = abmp_reader.read_var_int().unwrap() as u32;
        let tag = abmp_reader.read_u32().unwrap();

        // info!(
        //   "Found RIFX resource index {}: '{}', {} bytes ({} uncompressed) @ pos {}, compressionType: {}",
        //   res_id,
        //   fourcc_to_string(tag),
        //   comp_size,
        //   uncomp_size,
        //   offset,
        //   compression_type
        // );

        let info = ChunkInfo {
            id: res_id,
            fourcc: tag,
            len: comp_size,
            uncompressed_len: uncomp_size,
            offset: offset,
            compression_id: compression_ids[compression_type as usize],
        };
        chunk_info.insert(res_id, info);
    }

    // Initial load segment
    if !chunk_info.contains_key(&2) {
        return Err("readAfterburnerMap(): Map has no entry for ILS".to_owned());
    }
    if reader.read_u32().unwrap() != FOURCC("FGEI") {
        return Err("readAfterburnerMap(): FGEI expected but not found".to_owned());
    }

    let ils_info = chunk_info.get(&2).unwrap();
    let _ils_unk1 = reader.read_var_int().unwrap();
    // info!("ILS: length: {} unk1: {}", ils_info.len, ils_unk1);
    let ils_body_offset = reader.pos;

    let ils_uncomp = reader.read_zlib_bytes(ils_info.len).unwrap();
    if ils_uncomp.len() != ils_info.uncompressed_len {
        warn!(
            "ILS: Expected uncompressed length {} but got length {}",
            ils_info.uncompressed_len,
            ils_uncomp.len()
        );
    }

    let mut ils_reader = BinaryReader::from_vec(&ils_uncomp);
    ils_reader.set_endian(reader.endian);

    while !ils_reader.eof() {
        let res_id = ils_reader.read_var_int().unwrap() as u32;
        let info = chunk_info.get(&res_id).unwrap();

        // info!("Loading ILS resource {}: '{}', {} bytes", res_id, fourcc_to_string(info.fourcc), info.len);
        cached_chunk_views.insert(res_id, ils_reader.read_bytes(info.len).unwrap().to_vec());
    }
    return Ok(ils_body_offset);
}

fn read_key_table(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
) -> Result<KeyTableChunk, String> {
    let info = get_first_chunk_info(&chunk_container.chunk_info, FOURCC("KEY*"));

    match info {
        Some(info) => {
            let key_table = if let Chunk::KeyTable(key_table) =
                get_chunk(reader, chunk_container, rifx, info.fourcc, info.id).unwrap()
            {
                key_table
            } else {
                panic!("Not a keytable");
            };

            for i in 0..key_table.used_count {
                let entry = &key_table.entries[i as usize];
                let mut _owner_tag = FOURCC("????");
                if chunk_container.chunk_info.contains_key(&entry.cast_id) {
                    _owner_tag = chunk_container
                        .chunk_info
                        .get(&entry.cast_id)
                        .unwrap()
                        .fourcc;
                }
                // info!("KEY* entry ${i}: '{}' @ {} owned by '{}' @ {}", fourcc_to_string(entry.fourcc), entry.section_id, fourcc_to_string(owner_tag), entry.cast_id);
            }
            return Ok(key_table);
        }
        None => return Err("No key chunk!".to_owned()),
    }
}

fn get_first_chunk_info(chunk_info: &HashMap<u32, ChunkInfo>, fourcc: u32) -> Option<&ChunkInfo> {
    return chunk_info
        .iter()
        .find(|x| x.1.fourcc == fourcc)
        .map(|x| x.1);
}

fn get_first_chunk(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
) -> Option<Chunk> {
    let info = get_first_chunk_info(&chunk_container.chunk_info, fourcc);
    if info.is_some() {
        let info = info.unwrap();
        return Some(get_chunk(reader, chunk_container, rifx, info.fourcc, info.id).unwrap());
    } else {
        return None;
    }
}

fn read_chunk_data(reader: &mut BinaryReader, fourcc: u32, len: u32) -> Result<Vec<u8>, String> {
    let offset = reader.pos;

    let valid_fourcc = reader.read_u32().unwrap();
    let valid_len = reader.read_u32().unwrap();

    // use the valid length if mmap hasn't been read yet
    let mut use_len = len;
    if len == u32::MAX {
        use_len = valid_len;
    }

    // validate chunk
    if fourcc != valid_fourcc || use_len != valid_len {
        return Err(
      format_args!(
        "At offset ${offset} expected {} chunk with length ${use_len}, but got {} chunk with length ${valid_len}",
        fourcc_to_string(fourcc),
        fourcc_to_string(valid_fourcc),
      ).to_string()
    );
    } else {
        warn!(
            "At offset ${offset} reading chunk '{}' with length ${use_len}",
            fourcc_to_string(fourcc)
        );
    }

    return Ok(reader.read_bytes(use_len as usize).unwrap().to_vec());
}

pub fn read_director_file_bytes(
    bytes: &Vec<u8>,
    file_name: &str,
    base_path: &str,
) -> Result<DirectorFile, String> {
    let mut reader = binary_reader::BinaryReader::from_vec(bytes);

    return DirectorFile::read(
        file_name.to_owned(),
        Url::from_str(base_path).unwrap(),
        &mut reader,
    );
}

fn get_chunk_data(
    reader: &mut BinaryReader,
    chunk_container: &mut ChunkContainer,
    rifx: &RIFXReaderContext,
    fourcc: u32,
    id: u32,
) -> Result<Vec<u8>, String> {
    // let chunk_info = &mut self.chunk_info;
    // let cached_chunk_views = &self.cached_chunk_views;
    // let ils_body_offset = self.ils_body_offset;
    // let after_burned = self.after_burned;
    match chunk_container.chunk_info.get(&id) {
        Some(info) => {
            if fourcc != info.fourcc {
                return Err(format_args!(
                    "Expected chunk ${id} to be '{}', but is actually '{}'",
                    fourcc_to_string(fourcc),
                    fourcc_to_string(info.fourcc)
                )
                .to_string());
            }

            if chunk_container.cached_chunk_views.contains_key(&id) {
                return Ok(chunk_container
                    .cached_chunk_views
                    .get(&id)
                    .unwrap()
                    .to_vec());
            } else if rifx.after_burned {
                reader.jmp(info.offset + rifx.ils_body_offset);
                if info.len == 0 && info.uncompressed_len == 0 {
                    chunk_container
                        .cached_chunk_views
                        .insert(id, reader.read_bytes(info.len).unwrap().to_vec());
                } else if compression_implemented(&info.compression_id) {
                    let mut uncomp_buf: Option<Vec<u8>> = None;
                    if info.compression_id == ZLIB_COMPRESSION_GUID
                        || info.compression_id == ZLIB_COMPRESSION_GUID2
                    {
                        uncomp_buf = Some(reader.read_zlib_bytes(info.len).unwrap());
                    } else if info.compression_id == SND_COMPRESSION_GUID {
                        // Handle Director SND compressed chunk
                        reader.jmp(info.offset + rifx.ils_body_offset);

                        // Read raw bytes for this chunk
                        let snd_bytes = reader
                            .read_bytes(info.len)
                            .map_err(|e| format!("Failed to read SND chunk bytes: {}", e))?
                            .to_vec();

                        // Create a temporary BinaryReader over those bytes
                        let mut chunk_reader = BinaryReader::from_vec(&snd_bytes);
                        chunk_reader.endian = Endian::Big;

                        // Try to parse into a SoundChunk (your code)
                        match SoundChunk::from_snd_chunk(&mut chunk_reader) {
                            Ok(sound_chunk) => {
                                let wav_bytes = sound_chunk.to_wav(); // convert to usable PCM
                                chunk_container
                                    .cached_chunk_views
                                    .insert(id, wav_bytes.clone());
                                return Ok(wav_bytes);
                            }
                            Err(e) => {
                                warn!("Failed to parse SND chunk {}: {}", id, e);
                                // fallback: just insert the raw bytes to avoid crash
                                chunk_container
                                    .cached_chunk_views
                                    .insert(id, snd_bytes.to_vec());
                                return Ok(snd_bytes.to_vec());
                            }
                        }
                    }
                    if uncomp_buf.is_none() {
                        return Err(format!("Chunk ${id}: Could not decompress").to_string());
                    }
                    let uncomp_buf = uncomp_buf.unwrap();
                    if uncomp_buf.len() != info.uncompressed_len {
                        return Err(format_args!(
                            "Chunk ${id}: Expected uncompressed length {} but got length {}",
                            info.uncompressed_len,
                            uncomp_buf.len()
                        )
                        .to_string());
                    }
                    chunk_container
                        .cached_chunk_views
                        .insert(id, uncomp_buf.to_vec());
                } else if info.compression_id == FONTMAP_COMPRESSION_GUID {
                    warn!(
                        "FONTMAP compression not implemented — skipping chunk {}",
                        id
                    );

                    // Read and store raw bytes instead of panicking
                    let raw = reader
                        .read_bytes(info.len)
                        .map_err(|e| format!("Failed to read FONTMAP chunk {}: {}", id, e))?
                        .to_vec();

                    chunk_container.cached_chunk_views.insert(id, raw.clone());
                    return Ok(raw);
                } else {
                    if info.compression_id != NULL_COMPRESSION_GUID {
                        warn!("Unhandled compression type {}", info.compression_id)
                    }
                    chunk_container
                        .cached_chunk_views
                        .insert(id, reader.read_bytes(info.len).unwrap().to_vec());
                }
            } else {
                reader.jmp(info.offset);
                chunk_container
                    .cached_chunk_views
                    .insert(id, read_chunk_data(reader, fourcc, id).unwrap());
            }

            return Ok(chunk_container
                .cached_chunk_views
                .get(&id)
                .unwrap()
                .to_vec());
        }
        None => {
            Err(format_args!("Could not find chunk {} ${id}", fourcc_to_string(fourcc)).to_string())
        }
    }
}

pub fn get_chunk(
    reader: &mut BinaryReader,
    // endian: Endian,
    chunk_container: &mut ChunkContainer,
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
    id: u32,
) -> Result<Chunk, String> {
    // if deserialized_chunks.contains_key(&id) {
    //   return deserialized_chunks.get(&id).unwrap();
    // }

    let chunk_view = get_chunk_data(reader, chunk_container, rifx, fourcc, id);
    if let Ok(chunk_view) = chunk_view {
        let chunk = make_chunk(reader.endian, rifx, fourcc, &chunk_view);
        return chunk;
    } else {
        // warn!("Could not find chunk data for chunk {} of id {}", fourcc_to_string(fourcc), id);
        Err(chunk_view.unwrap_err())
    }
    // deserialized_chunks.insert(id, chunk);
    // return deserialized_chunks.get(&id).unwrap();
}

pub fn get_children_of_chunk<'a>(
    chunk_id: &u32,
    key_table: &'a KeyTableChunk,
) -> Vec<&'a KeyTableEntry> {
    let associations = key_table.entries.iter().filter(|x| x.cast_id == *chunk_id);
    return associations.collect_vec();
}

fn compression_implemented(compression_id: &MoaID) -> bool {
    return *compression_id == ZLIB_COMPRESSION_GUID
        || *compression_id == ZLIB_COMPRESSION_GUID2
        || *compression_id == SND_COMPRESSION_GUID;
}
