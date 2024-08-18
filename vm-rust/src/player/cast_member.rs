use core::fmt;
use std::{cell::RefCell, fmt::Formatter, rc::Rc};

use crate::director::{chunks::cast_member::CastMemberDef, enums::{MemberType, ScriptType, ShapeInfo}, lingo::script::ScriptContext};

use super::{bitmap::{bitmap::{decompress_bitmap, Bitmap, BuiltInPalette, PaletteRef, ProceduralBitmapType}, manager::{BitmapManager, BitmapRef}}, sprite::ColorRef, ScriptError};

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
  pub text_data: Rc<RefCell<TextData>>,
  pub image_ref: BitmapRef,
}

#[derive(Clone, Debug)]
pub struct TextData {
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
  pub fn new(bitmap_manager: &mut BitmapManager) -> TextMember {
    let width = 500;
    let font_size = 12;
    let text_data_rc = Rc::new(
      RefCell::new(
        TextData {
          text: "".to_string(),
          alignment: "left".to_string(),
          word_wrap: true,
          font: "Arial".to_string(),
          font_style: vec!["plain".to_string()],
          font_size,
          fixed_line_space: 0,
          top_spacing: 0,
          box_type: "adjust".to_string(),
          anti_alias: false,
          width,
        }
      )
    );
    TextMember {
      text_data: text_data_rc.clone(),
      image_ref: bitmap_manager.add_bitmap(
        Bitmap::new(
          width, 
          font_size, 
          8, 
          PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
          Some(ProceduralBitmapType::Text(text_data_rc, RefCell::new(true)))
        )
      ),
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

#[allow(dead_code)]
#[derive(Clone)]
pub enum CastMemberType {
  Field(FieldMember),
  Text(TextMember),
  Script(ScriptMember),
  Bitmap(BitmapMember),
  Palette(PaletteMember),
  Shape(ShapeMember),
  Unknown(u32)
}

#[derive(Debug)]
pub enum CastMemberTypeId {
  Field,
  Text,
  Script,
  Bitmap,
  Palette,
  Shape,
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
      Self::Unknown(id) => { write!(f, "Unknown({})", id) }
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
      Self::Unknown(_) => { CastMemberTypeId::Unknown }
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
              bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale), None))
            }
          }
        } else {
          bitmap_manager.add_bitmap(Bitmap::new(1, 1, 8, PaletteRef::BuiltIn(BuiltInPalette::GrayScale), None))
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
      _ => { 
        CastMemberType::Unknown(chunk.member_type_id)
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
