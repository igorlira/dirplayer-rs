pub mod config;
pub mod imap;
pub mod key_table;
pub mod cast_list;
pub mod list;
pub mod cast;
pub mod lctx;
pub mod cast_member;
pub mod cast_member_info;
pub mod script_names;
pub mod script;
pub mod literal;
pub mod handler;
pub mod score;
pub mod text;
pub mod bitmap;
pub mod palette;

use std::collections::HashMap;

use binary_reader::{Endian, BinaryReader};
use config::ConfigChunk;
use imap::InitialMapChunk;
use key_table::KeyTableChunk;
use score::FrameLabelsChunk;

use self::{bitmap::BitmapChunk, cast::CastChunk, cast_list::CastListChunk, cast_member::CastMemberChunk, lctx::ScriptContextChunk, palette::PaletteChunk, score::ScoreChunk, script::ScriptChunk, script_names::ScriptNamesChunk, text::TextChunk};
use super::{guid::MoaID, utils::{fourcc_to_string, FOURCC}, rifx::RIFXReaderContext};

pub struct CastInfoChunkProps {
}

pub struct MemoryMapChunkProps {
}

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
  Text(TextChunk),
  Bitmap(BitmapChunk),
  Palette(PaletteChunk),
}

impl Chunk {
  pub fn as_text(&self) -> Option<&TextChunk> {
    match self {
      Self::Text(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_bitmap(&self) -> Option<&BitmapChunk> {
    match self {
      Self::Bitmap(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_palette(&self) -> Option<&PaletteChunk> {
    match self {
      Self::Palette(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_score(&self) -> Option<&ScoreChunk> {
    match self {
      Self::Score(data) => { Some(data) }
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
}

#[allow(dead_code)]
pub fn is_chunk_writable(chunk_type: Chunk) -> bool {
  match chunk_type {
    Chunk::CastInfo(_) => { return true }
    Chunk::InitialMap(_) => { return true }
    Chunk::MemoryMap(_) => { return true }
    _ => { return false }
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
      return Ok(
        Chunk::InitialMap(
          InitialMapChunk::from_reader(&mut chunk_reader, version).unwrap()
        )
      );
    }
    // "mmap" => {
    //   //res = MemoryMapChunk(dir: this);
    // }
    "CAS*" => {
      return Ok(
        Chunk::Cast(
          CastChunk::from_reader(&mut chunk_reader, version).unwrap()
        )
      );
    }
    "CASt" => {
      return Ok(
        Chunk::CastMember(
          CastMemberChunk::from_reader(&mut chunk_reader, version).unwrap()
        )
      );
    }
    "KEY*" => {
      return Ok(
        Chunk::KeyTable(
          KeyTableChunk::from_reader(&mut chunk_reader, version).unwrap()
        )
      );
    }
    "LctX" | "Lctx" => {
      rifx.lctx_capital_x = fourcc == FOURCC("LctX");
      return Ok(
        Chunk::ScriptContext(
          ScriptContextChunk::from_reader(&mut chunk_reader, version).unwrap()
        )
      );
    }
    "Lnam" => {
      return Ok(
        Chunk::ScriptNames(
          ScriptNamesChunk::from_reader(&mut chunk_reader, version).unwrap()
        )
      );
    }
    "Lscr" => {
      return Ok(
        Chunk::Script(
          ScriptChunk::from_reader(&mut chunk_reader, version, rifx.lctx_capital_x).unwrap()
        )
      );
    }
    // "VWCF" => {
    //   //res = ConfigChunk(dir: this);
    // }
    "DRCF" => {
      return Ok(
        Chunk::Config(
          ConfigChunk::from_reader(&mut chunk_reader, version, endian).unwrap()
        )
      );
    }
    "MCsL" => {
      return Ok(
        Chunk::CastList(
          CastListChunk::from_reader(&mut chunk_reader, version, endian).unwrap()
        )
      )
      //res = CastListChunk(dir: this);
    }
    "VWSC" => {
      return Ok(
        Chunk::Score(
          ScoreChunk::read(&mut chunk_reader, version).unwrap()
        )
      )
    },
    "SCVW" => {
      return Ok(
        Chunk::Score(
          ScoreChunk::read(&mut chunk_reader, version).unwrap()
        )
      )
    }
    "VWLB" => {
      return Ok(
          Chunk::FrameLabels(
            FrameLabelsChunk::from_reader(&mut chunk_reader, version).unwrap()
        )
      )
    }
    "STXT" => {
      return Ok(
        Chunk::Text(
          TextChunk::read(&mut chunk_reader).unwrap()
        )
      )
    }
    "BITD" => {
      return Ok(
        Chunk::Bitmap(
          BitmapChunk::read(&mut chunk_reader).unwrap()
        )
      )
    }
    "CLUT" => Ok(Chunk::Palette(palette::PaletteChunk::from_reader(&mut chunk_reader, version).unwrap())),
    _ => {
      return Err(format_args!("Could not deserialize '{}' chunk", fourcc_to_string(fourcc)).to_string());
    }
  }
}
