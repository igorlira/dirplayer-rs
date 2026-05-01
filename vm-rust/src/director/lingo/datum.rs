use std::collections::VecDeque;
use std::borrow::Cow;
use std::sync::Arc;

use num_derive::FromPrimitive;

use crate::player::{
    DirPlayer, ScriptError, bitmap::{bitmap::PaletteRef, manager::BitmapRef, mask::BitmapMask}, cast_lib::CastMemberRef, cast_member::Media, datum_ref::DatumRef, script_ref::ScriptInstanceRef, sprite::{ColorRef, CursorRef}
};

#[allow(dead_code)]
#[derive(Clone, PartialEq, Debug)]
pub enum DatumType {
    Null,
    Void,
    Symbol,
    VarRef,
    ScriptInstanceRef,
    ScriptRef,
    CastLibRef,
    CastMemberRef,
    StageRef,
    SpriteRef,
    StringChunk,
    String,
    Int,
    Float,
    List,
    XmlChildNodes,
    ArgList,
    ArgListNoRet,
    PropList,
    Eval,
    Rect,
    Point,
    SoundRef,
    SoundChannel,
    CursorRef,
    TimeoutRef,
    TimeoutFactory,
    TimeoutInstance,
    ColorRef,
    BitmapRef,
    PaletteRef,
    Xtra,
    XtraInstance,
    Matte,
    PlayerRef,
    MovieRef,
    MouseRef,
    XmlRef,
    DateRef,
    MathRef,
    Vector,
    Media,
    JavaScript,
    FlashObjectRef,
    Shockwave3dObjectRef,
    Transform3d,
    HavokObjectRef,
}

#[derive(Clone, PartialEq, FromPrimitive)]
pub enum StringChunkType {
    Item,
    Word,
    Char,
    Line,
}

impl From<&str> for StringChunkType {
    fn from(s: &str) -> Self {
        match s {
            "item" | "items"  => StringChunkType::Item,
            "word" | "words"  => StringChunkType::Word,
            "char" | "chars" => StringChunkType::Char,
            "line" | "lines" => StringChunkType::Line,
            _ => panic!("Invalid string chunk type"),
        }
    }
}

impl From<&String> for StringChunkType {
    fn from(s: &String) -> Self {
        StringChunkType::from(s.as_str())
    }
}

impl From<&i32> for StringChunkType {
    fn from(n: &i32) -> Self {
        match n {
            // 0x01 => StringChunkType::Item,
            0x01 => StringChunkType::Char,
            0x02 => StringChunkType::Word,
            0x03 => StringChunkType::Item,
            // 0x03 => StringChunkType::Char,
            0x04 => StringChunkType::Line,
            _ => panic!("Invalid string chunk type"),
        }
    }
}

impl Into<String> for StringChunkType {
    fn into(self) -> String {
        match self {
            StringChunkType::Item => "item".to_string(),
            StringChunkType::Word => "word".to_string(),
            StringChunkType::Char => "char".to_string(),
            StringChunkType::Line => "line".to_string(),
        }
    }
}

#[derive(Clone)]
pub struct StringChunkExpr {
    pub chunk_type: StringChunkType,
    pub start: i32,
    pub end: i32,
    pub item_delimiter: char,
}

pub type PropListPair = (DatumRef, DatumRef);
pub type TimeoutRef = String;
pub type XtraInstanceId = u32;

#[derive(Clone, Debug)]
pub struct FlashObjectRef {
    pub path: String,
    pub instance_id: u32,
    pub cast_lib: i32,
    pub cast_member: i32,
}

/// Reference to a Shockwave 3D object (model, shader, texture, camera, light, group, motion).
/// The object_type + name identify the object within the member's W3dScene.
#[derive(Clone, Debug)]
pub struct Shockwave3dObjectRef {
    /// The cast member that owns this 3D object
    pub cast_lib: i32,
    pub cast_member: i32,
    /// Object type: "model", "shader", "texture", "camera", "light", "group", "motion", "modelResource"
    pub object_type: String,
    /// Object name within the scene
    pub name: String,
}

/// Reference to a Havok physics object (rigidBody, spring, linearDashpot, angularDashpot, corrector).
#[derive(Clone, Debug)]
pub struct HavokObjectRef {
    pub cast_lib: i32,
    pub cast_member: i32,
    /// "rigidBody", "spring", "linearDashpot", "angularDashpot", "corrector"
    pub object_type: String,
    /// Object name within the Havok scene
    pub name: String,
}

impl FlashObjectRef {
    pub fn from_path_with_member(path: &str, cast_lib: i32, cast_member: i32) -> Self {
        Self {
            path: path.to_string(),
            instance_id: 0,
            cast_lib,
            cast_member,
        }
    }

    pub fn from_path(path: &str) -> Self {
        Self {
            path: path.to_string(),
            instance_id: 0,
            cast_lib: 0,
            cast_member: 0,
        }
    }
}

#[derive(Clone)]
pub enum StringChunkSource {
    Datum(DatumRef),
    Member(CastMemberRef),
}

#[allow(dead_code)]
#[derive(Clone)]
pub enum Datum {
    Int(i32),
    Float(f64),
    String(String),
    StringChunk(StringChunkSource, StringChunkExpr, String),
    Void,
    VarRef(VarRef),
    List(DatumType, VecDeque<DatumRef>, bool), // bool is for whether the list is sorted
    PropList(VecDeque<PropListPair>, bool),    // bool is for whether the map is sorted
    Symbol(String),
    CastLib(u32),
    Stage,
    ScriptRef(CastMemberRef),
    ScriptInstanceRef(ScriptInstanceRef),
    CastMember(CastMemberRef),
    SpriteRef(i16),
    /// Inline rect: [left, top, right, bottom] with per-component int/float flags (bits 0-3).
    Rect([f64; 4], u8),
    /// Inline point: [x, y] with per-component int/float flags (bit 0 = x is float, bit 1 = y is float).
    Point([f64; 2], u8),
    SoundChannel(u16),
    CursorRef(CursorRef),
    TimeoutRef(TimeoutRef),
    TimeoutFactory,
    TimeoutInstance {
        name: String,
        duration: i32,
        callback: DatumRef,
        target: DatumRef,
        /// For script-based timeouts (like _TIMER_), this holds the script instance
        script_instance: Option<DatumRef>,
    },
    ColorRef(ColorRef),
    BitmapRef(BitmapRef),
    PaletteRef(PaletteRef),
    SoundRef(u16),
    Xtra(String),
    XtraInstance(String, XtraInstanceId),
    Matte(Arc<BitmapMask>),
    PlayerRef,
    MovieRef,
    MouseRef,
    XmlRef(u32),
    DateRef(u32),
    MathRef(u32),
    Vector([f64; 3]),
    Media(Media),
    Null,
    JavaScript(Vec<u8>),
    FlashObjectRef(FlashObjectRef),
    Shockwave3dObjectRef(Shockwave3dObjectRef),
    /// 4x4 row-major transform matrix for Shockwave 3D
    Transform3d([f64; 16]),
    HavokObjectRef(HavokObjectRef),
}

impl DatumType {
    pub fn type_str(&self) -> &'static str {
        match self {
            DatumType::Int => "int",
            DatumType::Float => "float",
            DatumType::String => "string",
            DatumType::StringChunk => "string_chunk",
            DatumType::Void => "void",
            DatumType::VarRef => "var_ref",
            DatumType::List => "list",
            DatumType::XmlChildNodes => "list",
            DatumType::PropList => "prop_list",
            DatumType::Symbol => "symbol",
            DatumType::CastLibRef => "cast_lib",
            DatumType::StageRef => "stage",
            DatumType::ScriptRef => "script_ref",
            DatumType::ScriptInstanceRef => "script_instance",
            DatumType::CastMemberRef => "cast_member",
            DatumType::SpriteRef => "sprite_ref",
            DatumType::Null => "null",
            DatumType::ArgList => "arg_list",
            DatumType::ArgListNoRet => "arg_list_no_ret",
            DatumType::Eval => "eval",
            DatumType::Rect => "rect",
            DatumType::Point => "point",
            DatumType::SoundChannel => "sound_channel",
            DatumType::SoundRef => "sound",
            DatumType::CursorRef => "cursor_ref",
            DatumType::TimeoutRef => "timeout",
            DatumType::TimeoutFactory => "timeout_factory",
            DatumType::TimeoutInstance => "timeout_instance",
            DatumType::ColorRef => "color_ref",
            DatumType::BitmapRef => "bitmap_ref",
            DatumType::PaletteRef => "palette_ref",
            DatumType::Xtra => "xtra",
            DatumType::XtraInstance => "xtra_instance",
            DatumType::Matte => "matte",
            DatumType::PlayerRef => "player_ref",
            DatumType::MovieRef => "movie_ref",
            DatumType::XmlRef => "xml",
            DatumType::DateRef => "date",
            DatumType::MathRef => "math",
            DatumType::Vector => "vector",
            DatumType::Media => "media",
            DatumType::JavaScript => "javascript",
            DatumType::FlashObjectRef => "flash_object_ref",
            DatumType::Shockwave3dObjectRef => "shockwave3d_object_ref",
            DatumType::MouseRef => "mouse_ref",
            DatumType::Transform3d => "transform",
            DatumType::HavokObjectRef => "havok_object_ref",
        }
    }
}

impl Datum {
    pub fn type_enum(&self) -> DatumType {
        match self {
            Datum::Int(_) => DatumType::Int,
            Datum::Float(_) => DatumType::Float,
            Datum::String(_) => DatumType::String,
            Datum::StringChunk(_, _, _) => DatumType::StringChunk,
            Datum::Void => DatumType::Void,
            Datum::VarRef(_) => DatumType::VarRef,
            Datum::List(_, _, _) => DatumType::List,
            Datum::PropList(..) => DatumType::PropList,
            Datum::Symbol(_) => DatumType::Symbol,
            Datum::CastLib(_) => DatumType::CastLibRef,
            Datum::Stage => DatumType::StageRef,
            Datum::ScriptRef(_) => DatumType::ScriptRef,
            Datum::ScriptInstanceRef(_) => DatumType::ScriptInstanceRef,
            Datum::CastMember(_) => DatumType::CastMemberRef,
            Datum::SpriteRef(_) => DatumType::SpriteRef,
            Datum::Rect(..) => DatumType::Rect,
            Datum::Point(..) => DatumType::Point,
            Datum::SoundChannel(_) => DatumType::SoundChannel,
            Datum::SoundRef(_) => DatumType::SoundRef,
            Datum::CursorRef(_) => DatumType::CursorRef,
            Datum::TimeoutRef(_) => DatumType::TimeoutRef,
            Datum::TimeoutFactory => DatumType::TimeoutRef,
            Datum::TimeoutInstance { .. } => DatumType::TimeoutRef,
            Datum::ColorRef(_) => DatumType::ColorRef,
            Datum::BitmapRef(_) => DatumType::BitmapRef,
            Datum::PaletteRef(_) => DatumType::PaletteRef,
            Datum::Xtra(_) => DatumType::Xtra,
            Datum::XtraInstance(..) => DatumType::XtraInstance,
            Datum::Matte(..) => DatumType::Matte,
            Datum::PlayerRef => DatumType::PlayerRef,
            Datum::MovieRef => DatumType::MovieRef,
            Datum::MouseRef => DatumType::MouseRef,
            Datum::XmlRef(_) => DatumType::XmlRef,
            Datum::DateRef(_) => DatumType::DateRef,
            Datum::MathRef(_) => DatumType::MathRef,
            Datum::Vector(_) => DatumType::Vector,
            Datum::Media(_) => DatumType::Media,
            Datum::Null => DatumType::Null,
            Datum::JavaScript(_) => DatumType::JavaScript,
            Datum::FlashObjectRef(_) => DatumType::FlashObjectRef,
            Datum::Shockwave3dObjectRef(_) => DatumType::Shockwave3dObjectRef,
            Datum::Transform3d(_) => DatumType::Transform3d,
            Datum::HavokObjectRef(_) => DatumType::HavokObjectRef,
        }
    }

    pub fn type_str(&self) -> &'static str {
        self.type_enum().type_str()
    }

    // TODO(zdimension): this should really return a Cow<str> instead of allocating a String
    pub fn string_value(&self) -> Result<String, ScriptError> {
        match self {
            Datum::String(s) => Ok(s.clone()),
            Datum::StringChunk(_, _, str_value) => Ok(str_value.to_owned()),
            Datum::Int(n) => Ok(n.to_string()),
            Datum::Float(n) => Ok(n.to_string()),
            Datum::Symbol(s) => Ok(s.clone()),
            Datum::Vector(v) => Ok(format!("[{},{},{}]", v[0], v[1], v[2])),
            Datum::Rect(r, f) => {
                let fmt = |i: usize| {
                    if Datum::inline_is_float(*f, i) { format!("{:.4}", r[i]) } else { format!("{}", r[i] as i32) }
                };
                Ok(format!("({}, {}, {}, {})", fmt(0), fmt(1), fmt(2), fmt(3)))
            },
            Datum::ColorRef(cr) => Ok(format!("{:?}", cr)),
            Datum::Void => Ok("VOID".to_string()),
            Datum::FlashObjectRef(fr) => Ok(fr.path.clone()),
            Datum::CastMember(member_ref) => Ok(format!(
                "(member {} of castLib {})", member_ref.cast_member, member_ref.cast_lib
            )),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum type {} to string",
                self.type_str()
            ))),
        }
    }

    pub fn string_value_cow(&self) -> Result<Cow<'_, str>, ScriptError> {
        match self {
            Datum::String(s) => Ok(Cow::Borrowed(s)),
            Datum::StringChunk(_, _, str_value) => Ok(Cow::Borrowed(str_value)),
            Datum::Int(n) => Ok(Cow::Owned(n.to_string())),
            Datum::Symbol(s) => Ok(Cow::Borrowed(s)),
            Datum::Vector(v) => Ok(Cow::Owned(format!("[{},{},{}]", v[0], v[1], v[2]))),
            Datum::Rect(r, f) => {
                let fmt = |i: usize| {
                    if Datum::inline_is_float(*f, i) { format!("{:.4}", r[i]) } else { format!("{}", r[i] as i32) }
                };
                Ok(Cow::Owned(format!("({}, {}, {}, {})", fmt(0), fmt(1), fmt(2), fmt(3))))
            },
            Datum::ColorRef(cr) => Ok(Cow::Owned(format!("{:?}", cr))),
            Datum::Void => Ok(Cow::Borrowed("VOID")),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum type {} to string",
                self.type_str()
            ))),
        }
    }

    pub fn symbol_value(&self) -> Result<String, ScriptError> {
        match self {
            Datum::Symbol(s) => Ok(s.clone()),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum type {} to symbol",
                self.type_str()
            ))),
        }
    }

    pub fn int_value(&self) -> Result<i32, ScriptError> {
        match self {
            Datum::Int(n) => Ok(*n),
            Datum::Float(n) => Ok(*n as i32),
            Datum::String(s) => Ok(s.parse().unwrap_or(0)),
            Datum::StringChunk(_, _, s) => Ok(s.parse().unwrap_or(0)),
            Datum::SpriteRef(n) => Ok(*n as i32),
            Datum::CastMember(member_ref) => Ok(member_ref.cast_member as i32),
            Datum::Symbol(_) => Ok(0),
            Datum::PaletteRef(_) => Ok(0),
            Datum::Void => Ok(0),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum of type {} to int",
                self.type_str()
            ))),
        }
    }

    pub fn float_value(&self) -> Result<f64, ScriptError> {
        match self {
            Datum::Float(n) => Ok(*n),
            Datum::Int(n) => Ok(*n as f64),
            Datum::String(s) => Ok(s.parse::<f64>().unwrap_or(0.0)),
            Datum::StringChunk(_, _, s) => Ok(s.parse::<f64>().unwrap_or(0.0)),
            Datum::SpriteRef(n) => Ok(*n as f64),
            Datum::Void => Ok(0.0),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum of type {} to float",
                self.type_str()
            ))),
        }
    }

    pub fn bool_value(&self) -> Result<bool, ScriptError> {
        match self {
            Datum::Int(n) => Ok(*n != 0),
            Datum::Float(n) => Ok(*n != 0.0),
            Datum::Symbol(..) => Ok(true),
            Datum::String(s) => Ok(!s.is_empty()),
            Datum::StringChunk(_, _, s) => Ok(!s.is_empty()),
            Datum::Void => Ok(false),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum of type {} to bool",
                self.type_str()
            ))),
        }
    }

    pub fn media_value(&self) -> Result<Media, ScriptError> {
        match self {
            Datum::Media(media) => Ok(media.clone()),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum of type {} to media",
                self.type_str()
            ))),
        }
    }

    pub fn is_number(&self) -> bool {
        match self {
            Datum::Int(_) => true,
            Datum::Float(_) => true,
            _ => false,
        }
    }

    pub fn is_int(&self) -> bool {
        match self {
            Datum::Int(_) => true,
            _ => false,
        }
    }

    pub fn is_string(&self) -> bool {
        match self {
            Datum::String(_) => true,
            Datum::StringChunk(..) => true,
            _ => false,
        }
    }

    pub fn is_symbol(&self) -> bool {
        match self {
            Datum::Symbol(_) => true,
            _ => false,
        }
    }

    pub fn is_list(&self) -> bool {
        match self {
            Datum::List(_, _, _) => true,
            _ => false,
        }
    }

    pub fn is_void(&self) -> bool {
        match self {
            Datum::Void => true,
            _ => false,
        }
    }

    pub fn is_flash_object(&self) -> bool {
        matches!(self, Datum::FlashObjectRef(_))
    }

    pub fn as_flash_object(&self) -> Option<&FlashObjectRef> {
        match self {
            Datum::FlashObjectRef(obj_ref) => Some(obj_ref),
            _ => None,
        }
    }

    pub fn to_float(&self) -> Result<f64, ScriptError> {
        match self {
            Datum::Int(n) => Ok(*n as f64),
            Datum::Float(n) => Ok(*n),
            Datum::String(s) => Ok(s.parse().unwrap_or(0.0)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to float".to_string(),
            )),
        }
    }

    pub fn to_bool(&self) -> Result<bool, ScriptError> {
        match self {
            Datum::Int(n) => Ok(*n != 0),
            Datum::Float(f) => Ok(*f != 0.0 && !f.is_nan()),
            Datum::String(s) => Ok(!s.is_empty()),
            Datum::Void | Datum::Null => Ok(false),
            Datum::Symbol(s) => {
                let lower = s.to_lowercase();
                Ok(lower != "false")
            },
            _ => Err(ScriptError::new("Cannot convert datum to bool".to_string())),
        }
    }

    pub fn to_list(&self) -> Result<&VecDeque<DatumRef>, ScriptError> {
        match self {
            Datum::List(_, items, _) => Ok(items),
            _ => Err(ScriptError::new("Cannot convert datum to list".to_string())),
        }
    }

    pub fn to_list_tuple(&self) -> Result<(&DatumType, &VecDeque<DatumRef>, bool), ScriptError> {
        match self {
            Datum::List(t, items, sorted) => Ok((t, items, *sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to list".to_string())),
        }
    }

    pub fn to_list_mut(
        &mut self,
    ) -> Result<(&mut DatumType, &mut VecDeque<DatumRef>, &mut bool), ScriptError> {
        match self {
            Datum::List(t, items, sorted) => Ok((t, items, sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to list".to_string())),
        }
    }

    #[allow(dead_code)]
    pub fn to_map(&self) -> Result<&VecDeque<(DatumRef, DatumRef)>, ScriptError> {
        match self {
            Datum::PropList(items, ..) => Ok(items),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_map_tuple(&self) -> Result<(&VecDeque<(DatumRef, DatumRef)>, bool), ScriptError> {
        match self {
            Datum::PropList(items, sorted) => Ok((items, *sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_map_mut(&mut self) -> Result<&mut VecDeque<(DatumRef, DatumRef)>, ScriptError> {
        match self {
            Datum::PropList(items, ..) => Ok(items),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_map_tuple_mut(
        &mut self,
    ) -> Result<(&mut VecDeque<(DatumRef, DatumRef)>, &mut bool), ScriptError> {
        match self {
            Datum::PropList(items, sorted) => Ok((items, sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_rect_inline(&self) -> Result<([f64; 4], u8), ScriptError> {
        match self {
            Datum::Rect(vals, flags) => Ok((*vals, *flags)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to rect".to_string(),
            )),
        }
    }

    pub fn to_rect_inline_mut(&mut self) -> Result<(&mut [f64; 4], &mut u8), ScriptError> {
        match self {
            Datum::Rect(vals, flags) => Ok((vals, flags)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to rect".to_string(),
            )),
        }
    }

    pub fn to_color_ref(&self) -> Result<&ColorRef, ScriptError> {
        match self {
            Datum::ColorRef(color) => Ok(color),
            _ => Err(ScriptError::new(
                "Cannot convert datum to color ref".to_string(),
            )),
        }
    }

    pub fn to_color_ref_mut(&mut self) -> Result<&mut ColorRef, ScriptError> {
        match self {
            Datum::ColorRef(color) => Ok(color),
            _ => Err(ScriptError::new(
                "Cannot convert datum to color ref".to_string(),
            )),
        }
    }

    pub fn to_sprite_ref(&self) -> Result<i16, ScriptError> {
        match self {
            Datum::SpriteRef(sprite_ref) => Ok(*sprite_ref),
            _ => Err(ScriptError::new(
                "Cannot convert datum to sprite ref".to_string(),
            )),
        }
    }

    pub fn to_point_inline(&self) -> Result<([f64; 2], u8), ScriptError> {
        match self {
            Datum::Point(vals, flags) => Ok((*vals, *flags)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to point".to_string(),
            )),
        }
    }

    pub fn to_point_inline_mut(&mut self) -> Result<(&mut [f64; 2], &mut u8), ScriptError> {
        match self {
            Datum::Point(vals, flags) => Ok((vals, flags)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to point".to_string(),
            )),
        }
    }

    pub fn to_bitmap_ref(&self) -> Result<&BitmapRef, ScriptError> {
        match self {
            Datum::BitmapRef(bitmap_ref) => Ok(bitmap_ref),
            _ => {
                let detail = match self {
                    Datum::Int(v) => format!("Int({})", v),
                    Datum::Float(v) => format!("Float({})", v),
                    Datum::String(v) => format!("String(\"{}\")", v),
                    Datum::Void => "Void".to_string(),
                    _ => format!("{:?}", self.type_enum()),
                };
                Err(ScriptError::new(
                    format!("Cannot convert {} to bitmap ref", detail),
                ))
            }
        }
    }

    pub fn to_xtra_instance(&self) -> Result<(&str, &XtraInstanceId), ScriptError> {
        match self {
            Datum::XtraInstance(xtra_name, xtra_instance) => Ok((xtra_name, xtra_instance)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to xtra instance".to_string(),
            )),
        }
    }

    pub fn to_xtra_name(&self) -> Result<&str, ScriptError> {
        match self {
            Datum::Xtra(name) => Ok(name),
            _ => Err(ScriptError::new(
                "Cannot convert datum to xtra name".to_string(),
            )),
        }
    }

    pub fn to_string_chunk(
        &self,
    ) -> Result<(&StringChunkSource, &StringChunkExpr, &str), ScriptError> {
        match self {
            Datum::StringChunk(datum_ref, expr, value) => Ok((datum_ref, expr, value)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to string chunk".to_string(),
            )),
        }
    }

    pub fn to_string_mut(&mut self) -> Result<&mut String, ScriptError> {
        // Coerce non-string types to String first (Lingo allows chunk ops on any value)
        match self {
            Datum::String(_) => {}
            _ => {
                let s = self.string_value()?;
                *self = Datum::String(s);
            }
        }
        match self {
            Datum::String(s) => Ok(s),
            _ => unreachable!(),
        }
    }

    pub fn to_mask(&self) -> Result<&BitmapMask, ScriptError> {
        match self {
            Datum::Matte(mask) => Ok(mask),
            _ => Err(ScriptError::new(format!(
                "Cannot convert datum of type {} to bitmap mask",
                self.type_str()
            ))),
        }
    }

    pub fn to_mask_or_none(&self) -> Option<&BitmapMask> {
        match self {
            Datum::Matte(mask) => Some(mask),
            _ => {
                log::error!(
                    "Cannot convert datum of type {} to bitmap mask. Returning None.",
                    self.type_str()
                );
                None
            }
        }
    }

    pub fn to_script_instance_ref(&self) -> Result<&ScriptInstanceRef, ScriptError> {
        match self {
            Datum::ScriptInstanceRef(id) => Ok(id),
            _ => Err(ScriptError::new(
                "Cannot convert datum to script instance id".to_string(),
            )),
        }
    }

    pub fn to_member_ref(&self) -> Result<CastMemberRef, ScriptError> {
        match self {
            Datum::CastMember(member_ref) => Ok(member_ref.clone()),
            _ => Err(ScriptError::new(
                "Cannot convert datum to cast member ref".to_string(),
            )),
        }
    }

    pub fn to_date_ref(&self) -> Result<u32, ScriptError> {
        match self {
            Datum::DateRef(id) => Ok(*id),
            _ => Err(ScriptError::new(
                "Cannot convert datum to date ref".to_string(),
            )),
        }
    }

    pub fn to_math_ref(&self) -> Result<u32, ScriptError> {
        match self {
            Datum::MathRef(id) => Ok(*id),
            _ => Err(ScriptError::new(
                "Cannot convert datum to math ref".to_string(),
            )),
        }
    }

    pub fn to_vector(&self) -> Result<[f64; 3], ScriptError> {
        match self {
            Datum::Vector(v) => Ok(*v),
            Datum::Int(v) => Ok([*v as f64, *v as f64, *v as f64]),
            Datum::Float(v) => Ok([*v, *v, *v]),
            Datum::Void => Ok([0.0, 0.0, 0.0]),
            _ => Err(ScriptError::new(format!(
                "Expected Vector, got {}",
                self.type_str()
            ))),
        }
    }

    pub fn to_f64(player: &DirPlayer, datum_ref: &DatumRef) -> Result<f64, ScriptError> {
        match player.get_datum(datum_ref) {
            Datum::Int(n) => Ok(*n as f64),
            Datum::Float(f) => Ok(*f),
            other => Err(ScriptError::new(format!(
                "Rect/Point component must be numeric, got {}",
                other.type_str()
            ))),
        }
    }

    pub fn from_f64(value: f64) -> Datum {
        if value.fract() == 0.0 {
            Datum::Int(value as i32)
        } else {
            Datum::Float(value as f64)
        }
    }

    /// Read an inline point/rect component back as a Datum, respecting int/float flag.
    pub fn inline_component_to_datum(val: f64, is_float: bool) -> Datum {
        if is_float {
            Datum::Float(val)
        } else {
            Datum::Int(val as i32)
        }
    }

    /// Convert a Datum to an inline component value and float flag.
    pub fn datum_to_inline_component(d: &Datum) -> Result<(f64, bool), ScriptError> {
        match d {
            Datum::Int(n) => Ok((*n as f64, false)),
            Datum::Float(f) => Ok((*f, true)),
            other => Err(ScriptError::new(format!(
                "Point/Rect component must be numeric, got {}",
                other.type_str()
            ))),
        }
    }

    /// Build a Point datum from two numeric Datums.
    pub fn build_point(x: &Datum, y: &Datum) -> Result<Datum, ScriptError> {
        let (xv, xf) = Datum::datum_to_inline_component(x)?;
        let (yv, yf) = Datum::datum_to_inline_component(y)?;
        let flags = (if xf { 1u8 } else { 0 }) | (if yf { 2u8 } else { 0 });
        Ok(Datum::Point([xv, yv], flags))
    }

    /// Build a Rect datum from four numeric Datums.
    pub fn build_rect(l: &Datum, t: &Datum, r: &Datum, b: &Datum) -> Result<Datum, ScriptError> {
        let (lv, lf) = Datum::datum_to_inline_component(l)?;
        let (tv, tf) = Datum::datum_to_inline_component(t)?;
        let (rv, rf) = Datum::datum_to_inline_component(r)?;
        let (bv, bf) = Datum::datum_to_inline_component(b)?;
        let flags = (if lf { 1u8 } else { 0 })
            | (if tf { 2u8 } else { 0 })
            | (if rf { 4u8 } else { 0 })
            | (if bf { 8u8 } else { 0 });
        Ok(Datum::Rect([lv, tv, rv, bv], flags))
    }

    /// Check if component i is float in an inline flags byte.
    pub fn inline_is_float(flags: u8, i: usize) -> bool {
        (flags & (1 << i)) != 0
    }

    /// Set or clear the float flag for component i.
    pub fn inline_set_float(flags: &mut u8, i: usize, is_float: bool) {
        if is_float {
            *flags |= 1 << i;
        } else {
            *flags &= !(1 << i);
        }
    }
}

pub fn datum_bool(val: bool) -> Datum {
    if val {
        DATUM_TRUE
    } else {
        DATUM_FALSE
    }
}

pub const DATUM_TRUE: Datum = Datum::Int(1);
pub const DATUM_FALSE: Datum = Datum::Int(0);

#[allow(dead_code)]
#[derive(Clone)]
pub enum VarRef {
    Script(CastMemberRef),
    ScriptInstance(ScriptInstanceRef),
}
