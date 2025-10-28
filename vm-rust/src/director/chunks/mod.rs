pub mod bitmap;
pub mod cast;
pub mod cast_info;
pub mod cast_list;
pub mod cast_member;
pub mod cast_member_info;
pub mod config;
pub mod effect;
pub mod handler;
pub mod imap;
pub mod key_table;
pub mod lctx;
pub mod list;
pub mod literal;
pub mod media;
pub mod palette;
pub mod pfr_renderer;
pub mod score;
pub mod score_order;
pub mod script;
pub mod script_names;
pub mod sound;
pub mod text;
pub mod thum;
pub mod xmedia;

use std::collections::HashMap;

use binary_reader::{BinaryReader, Endian};
use config::ConfigChunk;
use imap::InitialMapChunk;
use key_table::KeyTableChunk;
use score::FrameLabelsChunk;

use self::media::MediaChunk;
use self::score_order::SordChunk;
use self::sound::SoundChunk;
use self::{
    bitmap::BitmapChunk, cast::CastChunk, cast_list::CastListChunk, cast_member::CastMemberChunk,
    lctx::ScriptContextChunk, palette::PaletteChunk, score::ScoreChunk, script::ScriptChunk,
    script_names::ScriptNamesChunk, text::TextChunk,
};
use self::{cast_info::CastInfoChunk, effect::EffectChunk, thum::ThumChunk, xmedia::XMediaChunk};
use super::{
    guid::MoaID,
    rifx::RIFXReaderContext,
    utils::{fourcc_to_string, FOURCC},
};

pub struct CastInfoChunkProps {}

pub struct MemoryMapChunkProps {}

#[allow(dead_code)]
pub enum Chunk {
    Cast(CastChunk),
    CastList(CastListChunk),
    CastMember(CastMemberChunk),
    CastInfo(CastInfoChunkProps),
    Config(ConfigChunk),
    InitialMap(InitialMapChunk),
    KeyTable(KeyTableChunk),
    MemoryMap(MemoryMapChunkProps),
    Script(ScriptChunk),
    ScriptContext(ScriptContextChunk),
    ScriptNames(ScriptNamesChunk),
    FrameLabels(FrameLabelsChunk),
    Score(ScoreChunk),
    ScoreOrder(SordChunk),
    Text(TextChunk),
    Bitmap(BitmapChunk),
    Palette(PaletteChunk),
    Sound(SoundChunk),
    Media(MediaChunk),
    XMedia(XMediaChunk),
    CstInfo(CastInfoChunk),
    Effect(EffectChunk),
    Thum(ThumChunk),
}

impl Chunk {
    pub fn as_text(&self) -> Option<&TextChunk> {
        match self {
            Self::Text(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_bitmap(&self) -> Option<&BitmapChunk> {
        match self {
            Self::Bitmap(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_palette(&self) -> Option<&PaletteChunk> {
        match self {
            Self::Palette(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_score(&self) -> Option<&ScoreChunk> {
        match self {
            Self::Score(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_sound(&self) -> Option<&SoundChunk> {
        match self {
            Self::Sound(data) => Some(data),
            _ => None,
        }
    }
}

pub struct ChunkInfo {
    pub id: u32,
    pub fourcc: u32,
    pub len: usize,
    pub uncompressed_len: usize,
    pub offset: usize,
    pub compression_id: MoaID,
}

pub struct ChunkContainer {
    pub deserialized_chunks: HashMap<u32, Chunk>,
    pub chunk_info: HashMap<u32, ChunkInfo>,
    pub cached_chunk_views: HashMap<u32, Vec<u8>>,
}

#[allow(dead_code)]
pub fn is_chunk_writable(chunk_type: Chunk) -> bool {
    match chunk_type {
        Chunk::CastInfo(_) => return true,
        Chunk::InitialMap(_) => return true,
        Chunk::MemoryMap(_) => return true,
        _ => return false,
    }
}

pub fn make_chunk(
    endian: Endian,
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
    view: &Vec<u8>,
) -> Result<Chunk, String> {
    let version = rifx.dir_version;
    let mut chunk_reader = BinaryReader::from_vec(view);
    chunk_reader.set_endian(endian);

    match fourcc_to_string(fourcc).as_str() {
        "imap" => {
            return Ok(Chunk::InitialMap(InitialMapChunk::from_reader(
                &mut chunk_reader,
                version,
            )?));
        }
        // "mmap" => {
        //   //res = MemoryMapChunk(dir: this);
        // }
        "CAS*" => {
            return Ok(Chunk::Cast(CastChunk::from_reader(
                &mut chunk_reader,
                version,
            )?));
        }
        "CASt" => {
            return Ok(Chunk::CastMember(CastMemberChunk::from_reader(
                &mut chunk_reader,
                version,
            )?));
        }
        "KEY*" => {
            return Ok(Chunk::KeyTable(KeyTableChunk::from_reader(
                &mut chunk_reader,
                version,
            )?));
        }
        "LctX" | "Lctx" => {
            rifx.lctx_capital_x = fourcc == FOURCC("LctX");
            return Ok(Chunk::ScriptContext(ScriptContextChunk::from_reader(
                &mut chunk_reader,
                version,
            )?));
        }
        "Lnam" => {
            return Ok(Chunk::ScriptNames(ScriptNamesChunk::from_reader(
                &mut chunk_reader,
                version,
            )?));
        }
        "Lscr" => {
            return Ok(Chunk::Script(ScriptChunk::from_reader(
                &mut chunk_reader,
                version,
                rifx.lctx_capital_x,
            )?));
        }
        "DRCF" | "VWCF" => {
            return Ok(Chunk::Config(ConfigChunk::from_reader(
                &mut chunk_reader,
                version,
                endian,
            )?));
        }
        "MCsL" => {
            return Ok(Chunk::CastList(CastListChunk::from_reader(
                &mut chunk_reader,
                version,
                endian,
            )?));
            //res = CastListChunk(dir: this);
        }
        "VWSC" | "SCVW" => {
            return Ok(Chunk::Score(
                ScoreChunk::read(&mut chunk_reader, version).unwrap(),
            ))
        }
        "VWLB" => {
            return Ok(Chunk::FrameLabels(FrameLabelsChunk::from_reader(
                &mut chunk_reader,
                version,
            )?))
        }
        "ediM" => return Ok(Chunk::Media(MediaChunk::from_reader(&mut chunk_reader)?)),
        "Sord" => {
            return Ok(Chunk::ScoreOrder(SordChunk::from_reader(
                &mut chunk_reader,
            )?))
        }
        "snd " => return Ok(Chunk::Sound(SoundChunk::from_snd_chunk(&mut chunk_reader)?)),
        "STXT" => return Ok(Chunk::Text(TextChunk::read(&mut chunk_reader)?)),
        "BITD" => {
            return Ok(Chunk::Bitmap(BitmapChunk::read(
                &mut chunk_reader,
                version,
            )?))
        }
        "XMED" => return Ok(Chunk::XMedia(XMediaChunk::from_reader(&mut chunk_reader)?)),
        "Cinf" => {
            return Ok(Chunk::CstInfo(CastInfoChunk::from_reader(
                &mut chunk_reader,
            )?))
        }
        "FXmp" => return Ok(Chunk::Effect(EffectChunk::from_reader(&mut chunk_reader)?)),
        "Thum" => return Ok(Chunk::Thum(ThumChunk::from_reader(&mut chunk_reader)?)),
        "CLUT" => Ok(Chunk::Palette(palette::PaletteChunk::from_reader(
            &mut chunk_reader,
            version,
        )?)),
        _ => {
            return Err(
                format_args!("Could not deserialize '{}' chunk", fourcc_to_string(fourcc))
                    .to_string(),
            );
        }
    }
}
