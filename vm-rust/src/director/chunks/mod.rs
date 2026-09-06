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
pub mod pfr1;
pub mod score;
pub mod score_order;
pub mod tile_list;
pub mod script;
pub mod script_names;
pub mod sound;
pub mod text;
pub mod thum;
pub mod xmedia;
pub mod xtra_list;
pub mod w3d;
pub mod xmedia_styled_text;
pub mod cue_points;

use std::collections::HashMap;

use binary_reader::{BinaryReader, Endian};
use config::ConfigChunk;
use imap::InitialMapChunk;
use key_table::KeyTableChunk;
use score::FrameLabelsChunk;

use self::media::MediaChunk;
use self::score_order::SordChunk;
use self::sound::{SoundChunk, SndHeaderChunk};
use self::{
    bitmap::BitmapChunk, cast::CastChunk, cast_list::CastListChunk, cast_member::CastMemberChunk,
    lctx::ScriptContextChunk, palette::PaletteChunk, score::ScoreChunk, script::ScriptChunk,
    script_names::ScriptNamesChunk, text::TextChunk,
};
use self::{cast_info::CastInfoChunk, effect::EffectChunk, thum::ThumChunk, xmedia::XMediaChunk, xtra_list::XtraListChunk};
use self::cue_points::CuePointsChunk;
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
    TileList(tile_list::TileListChunk),
    Text(TextChunk),
    /// Plain text of a Rich Text Editor (RTE) cast member, stored in the
    /// member's RTE1 child chunk. RTE0 (style runs) is kept as Raw; the text
    /// (RTE1) and the pre-rendered bitmap (RTE2) are surfaced via these
    /// variants so the MemberType::RTE branch in cast_member.rs can build the
    /// member.
    RteText(String),
    /// Pre-rendered anti-aliased bitmap of an RTE member (RTE2 chunk): an
    /// 8-byte header (width/height BE u16 + flags) followed by a custom
    /// row-RLE of 4-bit coverage values. Decoded in cast_member.rs.
    RteBitmap(Vec<u8>),
    Bitmap(BitmapChunk),
    Palette(PaletteChunk),
    Sound(SoundChunk),
    SndHeader(SndHeaderChunk),
    SndSamples(Vec<u8>),
    Media(MediaChunk),
    XMedia(XMediaChunk),
    CstInfo(CastInfoChunk),
    Effect(EffectChunk),
    Thum(ThumChunk),
    XtraList(XtraListChunk),
    CuePoints(CuePointsChunk),
    Raw(Vec<u8>),
}

impl Chunk {
    pub fn as_text(&self) -> Option<&TextChunk> {
        match self {
            Self::Text(data) => Some(data),
            _ => None,
        }
    }

    pub fn as_rte_text(&self) -> Option<&str> {
        match self {
            Self::RteText(data) => Some(data.as_str()),
            _ => None,
        }
    }

    pub fn as_rte_bitmap(&self) -> Option<&[u8]> {
        match self {
            Self::RteBitmap(data) => Some(data.as_slice()),
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

    pub fn as_bytes(&self) -> Option<&Vec<u8>> {
        match self {
            Self::Raw(data) => { Some(data) }
            Self::SndSamples(data) => { Some(data) }
            Self::Media(m) => { Some(&m.audio_data) }
            Self::XMedia(x) => { Some(&x.raw_data) }
            _ => { None }
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
    /// Chunks that came out of the afterburner initial load segment. Their
    /// bytes live inside the ILS blob, not at `info.offset` in the file, so a
    /// dropped cache entry can NEVER be recovered for these. Every other chunk
    /// can be re-read and re-inflated on demand, which is what lets a big chunk
    /// hand its bytes to the deserialised form instead of being copied.
    pub ils_chunk_ids: std::collections::HashSet<u32>,
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

/// The chunk about to be built owns a full copy of the reader's buffer. Take it
/// when the caller allows, so the movie does not hold the same bytes twice.
fn own_buffer(chunk_reader: &mut BinaryReader, may_consume: bool) -> Vec<u8> {
    if may_consume {
        std::mem::take(&mut chunk_reader.data)
    } else {
        chunk_reader.data.clone()
    }
}

/// Deserialise one chunk from a borrowed buffer.
///
/// Copies the buffer, because `BinaryReader` owns its data and this caller only
/// has a borrow. Only the chunk inspector in `js_api` needs that; the movie
/// loader uses `make_chunk_in` and hands the bytes over by move.
pub fn make_chunk(
    endian: Endian,
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
    view: &Vec<u8>,
) -> Result<Chunk, String> {
    let mut chunk_reader = BinaryReader::from_vec(view);
    chunk_reader.set_endian(endian);
    make_chunk_in(rifx, fourcc, &mut chunk_reader, false)
}

/// Deserialise one chunk from a reader the caller already owns.
///
/// Split out so the movie loader never copies a chunk just to read it.
/// `BinaryReader::from_vec` clones its input, so building a reader per chunk was
/// a full second copy of every chunk in the file - for the measured movie that included
/// 29 MB of speech, copied once to cache it and again to parse it.
/// `may_consume`: the caller will not need the reader's buffer afterwards, so a
/// chunk that would otherwise copy the whole thing (media) may take it instead.
/// Only true for chunks that can be re-read from the file if asked for again.
pub fn make_chunk_in(
    rifx: &mut RIFXReaderContext,
    fourcc: u32,
    chunk_reader: &mut BinaryReader,
    may_consume: bool,
) -> Result<Chunk, String> {
    let version = rifx.dir_version;
    let chunk_reader_endian = chunk_reader.endian;

    match fourcc_to_string(fourcc).as_str() {
        "imap" => {
            return Ok(Chunk::InitialMap(InitialMapChunk::from_reader(
                chunk_reader,
                version,
            )?));
        }
        // "mmap" => {
        //   //res = MemoryMapChunk(dir: this);
        // }
        "CAS*" => {
            return Ok(Chunk::Cast(CastChunk::from_reader(
                chunk_reader,
                version,
            )?));
        }
        "CASt" => {
            return Ok(Chunk::CastMember(CastMemberChunk::from_reader(
                chunk_reader,
                version,
            )?));
        }
        "KEY*" => {
            return Ok(Chunk::KeyTable(KeyTableChunk::from_reader(
                chunk_reader,
                version,
            )?));
        }
        "LctX" | "Lctx" => {
            rifx.lctx_capital_x = fourcc == FOURCC("LctX");
            return Ok(Chunk::ScriptContext(ScriptContextChunk::from_reader(
                chunk_reader,
                version,
            )?));
        }
        "Lnam" => {
            return Ok(Chunk::ScriptNames(ScriptNamesChunk::from_reader(
                chunk_reader,
                version,
            )?));
        }
        "Lscr" => {
            return Ok(Chunk::Script(ScriptChunk::from_reader(
                chunk_reader,
                version,
                rifx.lctx_capital_x,
            )?));
        }
        "DRCF" | "VWCF" => {
            return Ok(Chunk::Config(ConfigChunk::from_reader(
                chunk_reader,
                version,
                chunk_reader_endian,
            )?));
        }
        "MCsL" => {
            return Ok(Chunk::CastList(CastListChunk::from_reader(
                chunk_reader,
                version,
                chunk_reader_endian,
            )?));
            //res = CastListChunk(dir: this);
        }
        "VWSC" | "SCVW" => {
            return Ok(Chunk::Score(
                ScoreChunk::read(chunk_reader, version, rifx.after_burned)?,
            ))
        }
        "VWLB" => {
            return Ok(Chunk::FrameLabels(FrameLabelsChunk::from_reader(
                chunk_reader,
                version,
            )?))
        }
        "ediM" => return Ok(Chunk::Media(MediaChunk::from_reader(chunk_reader)?)),
        "Sord" => {
            return Ok(Chunk::ScoreOrder(SordChunk::from_reader(
                chunk_reader,
            )?))
        }
        "VWTL" => {
            return Ok(Chunk::TileList(tile_list::TileListChunk::from_reader(
                chunk_reader,
                version,
            )?))
        }
        "snd " => return Ok(Chunk::Sound(SoundChunk::from_snd_chunk(chunk_reader, version)?)),
        "sndH" => return Ok(Chunk::SndHeader(SndHeaderChunk::from_reader(chunk_reader)?)),
        "sndS" => {
            // Sound samples chunk - just raw audio bytes
            log::debug!("sndS chunk: {} bytes of audio data", chunk_reader.data.len());
            return Ok(Chunk::SndSamples(own_buffer(chunk_reader, may_consume)));
        }
        "STXT" => return Ok(Chunk::Text(TextChunk::read(chunk_reader)?)),
        "RTE1" => {
            // Rich Text Editor text content — the raw text of an RTE member.
            // Labels are typically ASCII; decode leniently and drop a trailing
            // NUL terminator if present.
            let mut text = String::from_utf8_lossy(&chunk_reader.data).into_owned();
            if text.ends_with('\0') {
                text.truncate(text.trim_end_matches('\0').len());
            }
            return Ok(Chunk::RteText(text));
        }
        "RTE2" => {
            // RTE pre-rendered bitmap — keep raw; decoded in cast_member.rs.
            return Ok(Chunk::RteBitmap(own_buffer(chunk_reader, may_consume)));
        }
        "BITD" => {
            // The chunk owns the pixel data outright, so take the buffer rather
            // than cloning it: 271 bitmaps in the measured movie alone.
            return Ok(Chunk::Bitmap(BitmapChunk::from_data(
                own_buffer(chunk_reader, may_consume),
                version,
            )))
        }
        "XMED" => return Ok(Chunk::XMedia(XMediaChunk::from_reader(chunk_reader)?)),
        "Cinf" => {
            return Ok(Chunk::CstInfo(CastInfoChunk::from_reader(
                chunk_reader,
            )?))
        }
        "FXmp" => return Ok(Chunk::Effect(EffectChunk::from_reader(chunk_reader)?)),
        "Thum" => return Ok(Chunk::Thum(ThumChunk::from_reader(chunk_reader)?)),
        "XTRl" => return Ok(Chunk::XtraList(XtraListChunk::from_reader(chunk_reader)?)),
        "cupt" => return Ok(Chunk::CuePoints(CuePointsChunk::from_reader(chunk_reader)?)),
        "CLUT" => Ok(Chunk::Palette(palette::PaletteChunk::from_reader(
            chunk_reader,
            version,
        )?)),
        "ALFA" => {
            // Alpha channel data for JPEG bitmaps — store as raw bytes
            return Ok(Chunk::Raw(own_buffer(chunk_reader, may_consume)));
        }
        _ => {
            return Ok(Chunk::Raw(own_buffer(chunk_reader, may_consume)));
        }
    }
}

/// Hex preview of a byte buffer for `debug!`, capped and gated.
///
/// The log macros do gate on the level before evaluating their arguments, but
/// several chunk readers built a four-characters-per-byte String of the WHOLE
/// chunk in a SEPARATE `let` statement first, and that runs unconditionally.
/// The browser logger runs at Level::Error, so every one of those strings was
/// built and thrown away. On the measured movie's
/// 29 MB of speech (214 ediM chunks) that alone was 3.0 of the 3.2 seconds the
/// loading screen took. Returns empty unless debug logging is actually on, and
/// never formats more than `MAX` bytes.
pub fn hex_preview(data: &[u8]) -> String {
    const MAX: usize = 128;
    if !log::log_enabled!(log::Level::Debug) {
        return String::new();
    }
    let head = &data[..data.len().min(MAX)];
    let mut out = String::with_capacity(head.len() * 3 + 32);
    for b in head {
        out.push_str(&format!("{:02X} ", b));
    }
    if data.len() > MAX {
        out.push_str(&format!("... ({} bytes total)", data.len()));
    }
    out
}
