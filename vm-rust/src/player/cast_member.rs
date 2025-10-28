use core::fmt;
use std::fmt::Formatter;

use log::warn;

use crate::director::chunks::sound::SoundChunk;
use crate::director::enums::SoundInfo;

#[derive(Clone)]
pub struct CastMember {
  pub number: u32,
  pub name: String,
  pub member_type: CastMemberType,
  pub color: ColorRef,
  pub bg_color: ColorRef,
}

#[derive(Clone)]
pub struct FieldMember {
  pub text: String,
  pub alignment: String,
  pub word_wrap: bool,
  pub font: String,
  pub font_style: String,
  pub font_size: u16,
  pub fixed_line_space: u16,
  pub top_spacing: i16,
  pub box_type: String,
  pub anti_alias: bool,
  pub width: u16,
  pub auto_tab: bool, // Tabbing order depends on sprite number order, not position on the Stage.
  pub editable: bool,
  pub border: u16,
}

#[derive(Clone)]
pub struct TextMember {
  pub text: String,
  pub alignment: String,
  pub box_type: String,
  pub word_wrap: bool,
  pub anti_alias: bool,
  pub font: String,
  pub font_style: Vec<String>,
  pub font_size: u16,
  pub fixed_line_space: u16,
  pub top_spacing: i16,
  pub width: u16,
}

impl CastMember {
  pub fn new(number: u32, member_type: CastMemberType) -> CastMember {
    CastMember {
      number,
      name: "".to_string(),
      member_type,
      color: ColorRef::PaletteIndex(255),
      bg_color: ColorRef::PaletteIndex(0),
    }
  }
}

impl FieldMember {
  pub fn new() -> FieldMember {
    FieldMember {
      text: "".to_string(),
      alignment: "left".to_string(),
      word_wrap: true,
      font: "Arial".to_string(),
      font_style: "plain".to_string(),
      font_size: 12,
      fixed_line_space: 0,
      top_spacing: 0,
      box_type: "adjust".to_string(),
      anti_alias: false,
      width: 100,
      auto_tab: false,
      editable: false,
      border: 0,
    }
  }
}

impl TextMember {
  pub fn new() -> TextMember {
    TextMember {
      text: "".to_string(),
      alignment: "left".to_string(),
      word_wrap: true,
      font: "Arial".to_string(),
      font_style: vec!["plain".to_string()],
      font_size: 12,
      fixed_line_space: 0,
      top_spacing: 0,
      box_type: "adjust".to_string(),
      anti_alias: false,
      width: 100,
    }
  }
}

#[derive(Clone)]
pub struct ScriptMember {
  pub script_id: u32,
  pub script_type: ScriptType,
  pub name: String
}

#[derive(Clone)]
pub struct BitmapMember {
  pub image_ref: BitmapRef,
  pub reg_point: (i16, i16),
}

#[derive(Clone)]
pub struct PaletteMember {
  pub colors: Vec<(u8, u8, u8)>,
}

#[derive(Clone)]
pub struct ShapeMember {
  pub shape_info: ShapeInfo
}

impl PaletteMember {
  pub fn new() -> PaletteMember {
    PaletteMember {
      colors: vec![(0, 0, 0); 256],
    }
  }
}

#[derive(Clone)]
pub struct FilmLoopMember {
  pub info: FilmLoopInfo,
  pub score_chunk: ScoreChunk,
  pub score: Score,
}

#[derive(Clone)]
pub struct SoundMember {
  pub info: SoundInfo,
  pub sound: SoundChunk,
  // TODO add fields
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum CastMemberType {
  Field(FieldMember),
  Text(TextMember),
  Script(ScriptMember),
  Bitmap(BitmapMember),
  Palette(PaletteMember),
  Shape(ShapeMember),
  FilmLoop(FilmLoopMember),
  Sound(SoundMember),
  Unknown
}

#[derive(Debug)]
pub enum CastMemberTypeId {
  Field,
  Text,
  Script,
  Bitmap,
  Palette,
  Shape,
  FilmLoop,
  Sound,
  Unknown
}

impl fmt::Debug for CastMemberType {
  fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
    match self {
      Self::Field(_) => { write!(f, "Field") }
      Self::Text(_) => { write!(f, "Text") }
      Self::Script(_) => { write!(f, "Script") }
      Self::Bitmap(_) => { write!(f, "Bitmap") }
      Self::Palette(_) => { write!(f, "Palette") }
      Self::Shape(_) => { write!(f, "Shape") }
      Self::FilmLoop(_) => { write!(f, "FilmLoop") }
      Self::Sound(_) => { write!(f, "Sound") }
      Self::Unknown => { write!(f, "Unknown") }
    }
  }
}

impl CastMemberTypeId {
  pub fn symbol_string(&self) -> Result<&str, ScriptError> {
    return match self {
      Self::Field => { Ok("field") }
      Self::Text => { Ok("text") }
      Self::Script => { Ok("script") }
      Self::Bitmap => { Ok("bitmap") }
      Self::Palette => { Ok("palette") }
      Self::Shape => { Ok("shape") }
      Self::FilmLoop => { Ok("filmLoop") }
      Self::Sound => { Ok("sound") }
      _ => { Err(ScriptError::new("Unknown cast member type".to_string())) }
    }
  }
}

impl CastMemberType {
  pub fn member_type_id(&self) -> CastMemberTypeId {
    return match self {
      Self::Field(_) => { CastMemberTypeId::Field }
      Self::Text(_) => { CastMemberTypeId::Text }
      Self::Script(_) => { CastMemberTypeId::Script }
      Self::Bitmap(_) => { CastMemberTypeId::Bitmap }
      Self::Palette(_) => { CastMemberTypeId::Palette }
      Self::Shape(_) => { CastMemberTypeId::Shape }
      Self::FilmLoop(_) => { CastMemberTypeId::FilmLoop }
      Self::Sound(_) => { CastMemberTypeId::Sound }
      Self::Unknown => { CastMemberTypeId::Unknown }
    }
  }

  pub fn type_string(&self) -> &str {
    return match self {
      Self::Field(_) => { "field" }
      Self::Text(_) => { "text" }
      Self::Script(_) => { "script" }
      Self::Bitmap(_) => { "bitmap" }
      Self::Palette(_) => { "palette" }
      Self::Shape(_) => { "shape" }
      Self::FilmLoop(_) => { "filmLoop" }
      Self::Sound(_) => { "sound" }
      _ => { "unknown" }
    }
  }

  #[allow(dead_code)]
  pub fn as_script(&self) -> Option<&ScriptMember> {
    return match self {
      Self::Script(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_field(&self) -> Option<&FieldMember> {
    return match self {
      Self::Field(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_field_mut(&mut self) -> Option<&mut FieldMember> {
    return match self {
      Self::Field(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_text(&self) -> Option<&TextMember> {
    return match self {
      Self::Text(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_text_mut(&mut self) -> Option<&mut TextMember> {
    return match self {
      Self::Text(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_bitmap(&self) -> Option<&BitmapMember> {
    return match self {
      Self::Bitmap(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_bitmap_mut(&mut self) -> Option<&mut BitmapMember> {
    return match self {
      Self::Bitmap(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_palette(&self) -> Option<&PaletteMember> {
    return match self {
      Self::Palette(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_film_loop(&self) -> Option<&FilmLoopMember> {
    return match self {
      Self::FilmLoop(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_film_loop_mut(&mut self) -> Option<&mut FilmLoopMember> {
    return match self {
      Self::FilmLoop(data) => { Some(data) }
      _ => { None }
    }
  }

  pub fn as_sound(&self) -> Option<&SoundMember> {
    return match self {
      Self::Sound(data) => { Some(data) }
      _ => { None }
    }
  }
}

impl CastMember {
  pub fn from(
    cast_lib: u32,
    number: u32, 
    member_def: &CastMemberDef, 
    lctx: &Option<ScriptContext>,
    bitmap_manager: &mut BitmapManager,
  ) -> CastMember {
    let chunk = &member_def.chunk;

    let member_type = match chunk.member_type {
      MemberType::Text => {
        let text_chunk = member_def.children[0].as_ref().unwrap().as_text().expect("Not a text chunk");
        let mut field_member = FieldMember::new();
        field_member.text = text_chunk.text.clone();
        CastMemberType::Field(field_member)
      }
      MemberType::Script => {
        let member_info = chunk.member_info.as_ref().unwrap();
        let script_id = member_info.header.script_id;
        let script_type = chunk.specific_data.script_type().unwrap();
        let _script_chunk = &lctx.as_ref().unwrap().scripts[&script_id];

        CastMemberType::Script(
          ScriptMember { 
            script_id, 
            script_type, 
            name: member_info.name.clone() 
          }
        )
      }
      MemberType::Bitmap => {
        let bitmap_info = chunk.specific_data.bitmap_info().unwrap();
        let abmp_chunk = member_def.children
          .get(0)
          .and_then(|x| x.as_ref());
        let new_bitmap_ref = if let Some(abmp_chunk) = abmp_chunk {
          let abmp_chunk = abmp_chunk
            .as_bitmap()
            .unwrap();
          let decompressed = decompress_bitmap(&abmp_chunk.data, &bitmap_info, cast_lib);
          match decompressed {
            Ok(new_bitmap) => {
              bitmap_manager.add_bitmap(new_bitmap)
            },
            Err(_e) => {
              // warn!("Failed to decompress bitmap. Using an empty image instead. {:?}", e);
              // TODO create error texture?
              // INVALID_BITMAP_REF
              bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))
            }
          }
        } else {
          warn!("No bitmap chunk found for member {}", number);
          bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))
        };

        CastMemberType::Bitmap(
          BitmapMember {
            image_ref: new_bitmap_ref,
            reg_point: (bitmap_info.reg_x, bitmap_info.reg_y),
          }
        )
      }
      MemberType::Palette => {
        let palette_chunk = member_def.children[0].as_ref().unwrap().as_palette().expect("Not a palette chunk");
        CastMemberType::Palette(PaletteMember { colors: palette_chunk.colors.clone() })
      }
      MemberType::Shape => {
        CastMemberType::Shape(ShapeMember {
          shape_info: chunk.specific_data.shape_info().unwrap().clone()
        })
      }
      MemberType::FilmLoop => {
        let score_chunk = member_def.children[0].as_ref().unwrap().as_score().unwrap();
        let film_loop_info = chunk.specific_data.film_loop_info().unwrap();
        let mut score = Score::empty();
        score.load_from_score_chunk(score_chunk);
        CastMemberType::FilmLoop(FilmLoopMember {
          info: film_loop_info.clone(),
          score_chunk: score_chunk.clone(),
          score,
        })
      }
      MemberType::Sound => {
        // Log children
        if !member_def.children.is_empty() {
          console::log_1(&format!(
            "CastMember {} has {} children:",
            number,
            member_def.children.len()
          ).into());

          for (i, c_opt) in member_def.children.iter().enumerate() {
            match c_opt {
              Some(c) => console::log_1(&format!("child[{}] = {}", i, Self::chunk_type_name(c)).into()),
              None => console::log_1(&format!("child[{}] = None", i).into()),
            }
          }
        }

        // Try to find a sound chunk
        let sound_chunk_opt = member_def.children.iter()
          .filter_map(|c_opt| c_opt.as_ref())
          .find_map(|chunk| match chunk {
            Chunk::Sound(s) => {
              console::log_1(&format!("Found Sound chunk with {} bytes", s.data().len()).into());
              Some(s.clone())
            },
            Chunk::Media(m) => {
              console::log_1(&format!(
                "Found Media chunk: sample_rate={}, data_size_field={}, audio_data.len()={}, is_compressed={}",
                m.sample_rate, m.data_size_field, m.audio_data.len(), m.is_compressed
              ).into());
              
              // Check if the Media chunk has any sound data
              // Don't just check is_empty - also check data_size_field
              if !m.audio_data.is_empty() || m.data_size_field > 0 {
                let sound = SoundChunk::from_media(&m);
                console::log_1(&format!(
                  "Created SoundChunk from Media: {} bytes, rate={}",
                  sound.data().len(), sound.sample_rate()
                ).into());
                Some(sound)
              } else {
                console::log_1(&"Media chunk has no audio data".into());
                None
              }
            },
            _ => None,
          });

        let found_sound = sound_chunk_opt.is_some();
        console::log_1(&format!(
          "CastMember {}: {} children, found sound chunk = {}",
          number,
          member_def.children.len(),
          found_sound
        ).into());

        // Construct SoundMember
        if let Some(sound_chunk) = sound_chunk_opt {
          let info = SoundInfo {
            sample_rate: sound_chunk.sample_rate(),
            sample_size: sound_chunk.bits_per_sample(),
            channels: sound_chunk.channels(),
            sample_count: sound_chunk.sample_count(),
            duration: if sound_chunk.sample_rate() > 0 {
              (sound_chunk.sample_count() as f32 / sound_chunk.sample_rate() as f32 * 1000.0).round() as u32
            } else {
              0
            },
          };

          console::log_1(&format!(
            "SoundMember created → sample_rate: {}, sample_size: {}, channels: {}, sample_count: {}, duration: {:.3}s",
            info.sample_rate,
            info.sample_size,
            info.channels,
            info.sample_count,
            info.duration
          ).into());

          CastMemberType::Sound(SoundMember {
            info,
            sound: sound_chunk,
          })
        } else {
          warn!("No sound chunk found for member {}", number);
          CastMemberType::Sound(SoundMember {
            info: SoundInfo::default(),
            sound: SoundChunk::default(),
          })
        }
      }
      _ => { 
        CastMemberType::Unknown
      }
    };
    CastMember {
      number,
      name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
      member_type: member_type,
      color: ColorRef::PaletteIndex(255),
      bg_color: ColorRef::PaletteIndex(0),
    }
  }
}
