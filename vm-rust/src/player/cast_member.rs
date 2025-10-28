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

#[derive(Clone, Default)]
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
}

#[derive(Clone, Debug)]
pub struct FontMember {
  pub font_info: FontInfo,
  pub preview_text: String,
  pub preview_html_spans: Vec<StyledSpan>,
  pub fixed_line_space: u16,
  pub top_spacing: i16,
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
  Font(FontMember),
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
  Font,
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
      Self::Font(_) => { write!(f, "Font") }
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
      Self::Font => { Ok("Font") }
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
      Self::Font(_) => { CastMemberTypeId::Font }
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
      Self::Font(_) => { "font" }
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

  pub fn as_font(&self) -> Option<&FontMember> {
    return match self {
      Self::Font(data) => { Some(data) }
      _ => { None }
    }
  }
}

impl CastMember {
  fn chunk_type_name(c: &Chunk) -> &'static str {
      match c {
        Chunk::Cast(_) => "Cast",
        Chunk::CastList(_) => "CastList",
        Chunk::CastMember(_) => "CastMember",
        Chunk::CastInfo(_) => "CastInfo",
        Chunk::Config(_) => "Config",
        Chunk::InitialMap(_) => "InitialMap",
        Chunk::KeyTable(_) => "KeyTable",
        Chunk::MemoryMap(_) => "MemoryMap",
        Chunk::Script(_) => "Script",
        Chunk::ScriptContext(_) => "ScriptContext",
        Chunk::ScriptNames(_) => "ScriptNames",
        Chunk::FrameLabels(_) => "FrameLabels",
        Chunk::Score(_) => "Score",
        Chunk::ScoreOrder(_) => "ScoreOrder",
        Chunk::Text(_) => "Text",
        Chunk::Bitmap(_) => "Bitmap",
        Chunk::Palette(_) => "Palette",
        Chunk::Sound(_) => "Sound",
        Chunk::Media(_) => "Media",
      }
  }

  /// Recursively searches children of a CastMemberDef for a sound chunk
  fn find_sound_chunk_in_def(def: &CastMemberDef) -> Option<SoundChunk> {
    for child_opt in &def.children {
      if let Some(child) = child_opt {
        match child {
          Chunk::Sound(s) => return Some(s.clone()),
          Chunk::Media(m) => {
            if !m.audio_data.is_empty() {
              let mut sc = SoundChunk::new(m.audio_data.clone());
              sc.set_metadata(m.sample_rate, 1, if m.is_compressed { 0 } else { 16 });
              return Some(sc);
            }
          }
          Chunk::CastMember(_) => {
            // `CastMemberChunk` has no children, so nothing to recurse into
            continue;
          }
          _ => {}
        }
      }
    }
    None
  }

  fn child_has_sound_in_def(def: &CastMemberDef) -> bool {
    def.children.iter().any(|c| {
      match c {
        Some(Chunk::Sound(_)) => true,
        Some(Chunk::Media(m)) => !m.audio_data.is_empty(),
        Some(Chunk::CastMember(_)) => false,
        _ => false,
      }
    })
  }

  /// Recursively find a SoundChunk in a Chunk (handles Media & nested CastMembers)
  fn find_sound_chunk_in_chunk(chunk: &Chunk) -> Option<SoundChunk> {
    match chunk {
      Chunk::Sound(s) => Some(s.clone()),
      Chunk::Media(m) if !m.audio_data.is_empty() => {
        let mut sc = SoundChunk::new(m.audio_data.clone());
        sc.set_metadata(m.sample_rate, 1, if m.is_compressed { 0 } else { 16 });
        Some(sc)
      }
      Chunk::CastMember(cm) => {
        // CastMemberChunk has no children; nothing to recurse
        None
      }
      _ => None,
    }
  }

  // Check if an Option<Chunk> contains sound
  fn chunk_has_sound(chunk_opt: &Option<Chunk>) -> bool {
    match chunk_opt {
      Some(c) => match c {
        Chunk::Sound(_) => true,
        Chunk::Media(m) => !m.audio_data.is_empty(),
        _ => false,
      },
      None => false,
    }
  }

  // Extract SoundChunk from an Option<Chunk>
  fn find_sound_chunk(chunk_opt: &Option<Chunk>) -> Option<SoundChunk> {
    match chunk_opt {
      Some(c) => match c {
        Chunk::Sound(s) => Some(s.clone()),
        Chunk::Media(m) => {
          if !m.audio_data.is_empty() {
            Some(SoundChunk::from_media(m))
          } else {
            None
          }
        }
        _ => None,
      },
      None => None,
    }
  }

  fn decode_bitmap_from_bitd(
    member_def: &CastMemberDef,
    bitmap_info: &BitmapInfo,
    cast_lib: u32,
    number: u32,
    bitmap_manager: &mut BitmapManager,
  ) -> BitmapRef {
    let abmp_chunk = member_def.children
      .get(0)
      .and_then(|x| x.as_ref());
    
    if let Some(abmp_chunk) = abmp_chunk {
      let abmp_chunk = abmp_chunk.as_bitmap().unwrap();
      let decompressed = decompress_bitmap(
        &abmp_chunk.data, 
        &bitmap_info, 
        cast_lib,
        abmp_chunk.version
      );
      match decompressed {
        Ok(new_bitmap) => {
          bitmap_manager.add_bitmap(new_bitmap)
        },
        Err(e) => {
          warn!("Failed to decompress bitmap {}: {:?}. Using empty image.", number, e);
          bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, 0, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))
        }
      }
    } else {
      warn!("No bitmap chunk found for member {}", number);
      bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, 0, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))
    }
  }

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
      MemberType::Font => {
        use web_sys::console;
        
        let font_name = chunk
          .member_info
          .as_ref()
          .map(|info| info.name.clone())
          .unwrap_or_else(|| "<unnamed>".to_string());

        // Try to get font_info from specific_data
        let font_info_opt = chunk.specific_data.font_info();
        
        // Get raw specific data for analysis
        let specific_data_bytes = chunk.specific_data_raw.clone();

        // CRITICAL: Check if this looks like real font data
        let has_font_binary_data = FontInfo::looks_like_real_font_data(&specific_data_bytes);
        let is_text_data = FontInfo::looks_like_text_data(&specific_data_bytes);

        if is_text_data {
          if let Some(text_data) = TextMemberData::from_raw_bytes(&specific_data_bytes) {
            text_data.log_summary();
            
            // // Convert to TextMember
            // let mut text_member = TextMember::new();
            // text_member.alignment = text_data.alignment_string().to_string();
            // text_member.font_size = text_data.font_size;
            // text_member.word_wrap = text_data.word_wrap;
            // text_member.anti_alias = text_data.anti_alias;
            // text_member.box_type = text_data.box_type_string().to_string();
            // // ... set other fields
          }
        }
    
        if is_text_data || (!has_font_binary_data && font_info_opt.is_none()) {
          console::log_1(&format!(
              "   ❌ This is {} - converting to Text member",
              if is_text_data { "text data" } else { "not font data" }
          ).into());
          
          // Try to treat as Text member
          if member_def.children.len() > 0 {
            if let Some(first_child) = &member_def.children[0] {
              if let Some(text_chunk) = first_child.as_text() {
                console::log_1(&format!(
                  "   → Converting to Text member (text: '{}')",
                  &text_chunk.text.chars().take(50).collect::<String>()
                ).into());
                
                let mut text_member = TextMember::new();
                text_member.text = text_chunk.text.clone();
                
                return CastMember {
                  number,
                  name: font_name,
                  member_type: CastMemberType::Text(text_member),
                  color: ColorRef::PaletteIndex(255),
                  bg_color: ColorRef::PaletteIndex(0),
                };
              }
            }
          }
          
          console::log_1(&"   → No text chunk, marking as Unknown".into());
          return CastMember {
            number,
            name: font_name,
            member_type: CastMemberType::Unknown,
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
          };
        }

        // Get or create FontInfo
        let font_info = if let Some(info) = font_info_opt {
          info.clone()
        } else if has_font_binary_data {
          // Try to parse from raw bytes with FourCC
          if let Some(mut info) = FontInfo::from_raw_with_fourcc(&specific_data_bytes) {
            // Set name from member_info
            info.name = font_name.clone();
            info
          } else {
            // Fallback
            FontInfo {
              font_id: 0,
              size: 12,
              style: 0,
              name: font_name.clone(),
            }
          }
        } else {
          // Parse from raw bytes (old format)
          let mut info = FontInfo::from(specific_data_bytes.as_slice());
          info.name = font_name.clone();
          info
        };
        
        // Additional validation: check if font name makes sense
        let name_looks_like_font = {
          let name = font_info.name.trim();
          let has_text_indicators = name.to_lowercase().contains("text") ||
            name.to_lowercase().contains("label") ||
            name.to_lowercase().contains("box") ||
            name.to_lowercase().contains("button") ||
            name.to_lowercase().contains("field");
          
          !has_text_indicators || name.chars().any(|c| c == '*' || c == '-')
        };
        
        if !name_looks_like_font && !has_font_binary_data {
          console::log_1(&format!(
            "   ⚠️  Name '{}' doesn't look like a font name",
            font_info.name
          ).into());
        }

        console::log_1(&format!(
          "   ✅ Valid Font: id={}, size={}, style={}, name='{}' (member #{})",
          font_info.font_id,
          font_info.size,
          font_info.style,
          font_info.name,
          number
        ).into());

        CastMemberType::Font(FontMember {
          font_info,
          preview_text: "".to_string(),
          preview_html_spans: Vec::new(),
          fixed_line_space: 0,
          top_spacing: 0,
        })
      }
      MemberType::Bitmap => {
        let bitmap_info = chunk.specific_data.bitmap_info().unwrap();
        
        // First, check if there's a Media (ediM) chunk with JPEG data
        let media_chunk = member_def.children.iter()
          .find_map(|c| c.as_ref().and_then(|chunk| {
            match chunk {
              Chunk::Media(m) => Some(m),
              _ => None,
            }
          }));
        
        let new_bitmap_ref = if let Some(media) = media_chunk {
          // Check if the media chunk contains JPEG data
          let is_jpeg = if media.audio_data.len() >= 4 {
            let header = u32::from_be_bytes([
              media.audio_data[0],
              media.audio_data[1],
              media.audio_data[2],
              media.audio_data[3],
            ]);
            // JPEG magic numbers: FFD8FFE0, FFD8FFE1, FFD8FFE2, FFD8FFDB
            (header & 0xFFFFFF00) == 0xFFD8FF00
          } else {
            false
          };
          
          if is_jpeg && !media.audio_data.is_empty() {
            web_sys::console::log_1(&format!(
              "Found JPEG data in Media chunk for bitmap {}, size: {} bytes",
              number, media.audio_data.len()
            ).into());
            
            match decode_jpeg_bitmap(&media.audio_data, &bitmap_info) {
              Ok(new_bitmap) => {
                web_sys::console::log_1(&format!(
                    "Successfully decoded JPEG: {}x{}, bit_depth: {}",
                    new_bitmap.width, new_bitmap.height, new_bitmap.bit_depth
                ).into());
                bitmap_manager.add_bitmap(new_bitmap)
              },
              Err(e) => {
                warn!("Failed to decode JPEG bitmap {}: {:?}. Using empty image.", number, e);
                bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, 0, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale)))
              }
            }
          } else {
            // Media chunk exists but doesn't contain JPEG, fall back to BITD
            Self::decode_bitmap_from_bitd(member_def, &bitmap_info, cast_lib, number, bitmap_manager)
          }
        } else {
          // No Media chunk, use BITD
          Self::decode_bitmap_from_bitd(member_def, &bitmap_info, cast_lib, number, bitmap_manager)
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
