use std::sync::Arc;

use num_derive::FromPrimitive;

use crate::player::{
    bitmap::{bitmap::PaletteRef, manager::BitmapRef, mask::BitmapMask},
    cast_lib::CastMemberRef,
    datum_ref::DatumRef,
    script_ref::ScriptInstanceRef,
    sprite::{ColorRef, CursorRef},
    ScriptError, DirPlayer,
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
    XmlRef,
    DateRef,
    MathRef,
    Vector,
}

#[derive(Clone, PartialEq, FromPrimitive)]
pub enum StringChunkType {
    Item,
    Word,
    Char,
    Line,
}

impl From<&String> for StringChunkType {
    fn from(s: &String) -> Self {
        match s.as_str() {
            "item" => StringChunkType::Item,
            "word" => StringChunkType::Word,
            "char" => StringChunkType::Char,
            "line" => StringChunkType::Line,
            _ => panic!("Invalid string chunk type"),
        }
    }
}

impl From<&i32> for StringChunkType {
    fn from(n: &i32) -> Self {
        match n {
            0x01 => StringChunkType::Item,
            0x02 => StringChunkType::Word,
            0x03 => StringChunkType::Char,
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
    List(DatumType, Vec<DatumRef>, bool), // bool is for whether the list is sorted
    PropList(Vec<PropListPair>, bool),    // bool is for whether the map is sorted
    Symbol(String),
    CastLib(u32),
    Stage,
    ScriptRef(CastMemberRef),
    ScriptInstanceRef(ScriptInstanceRef),
    CastMember(CastMemberRef),
    SpriteRef(i16),
    Rect([DatumRef; 4]),
    Point([DatumRef; 2]),
    SoundChannel(u16),
    CursorRef(CursorRef),
    TimeoutRef(TimeoutRef),
    TimeoutFactory,
    TimeoutInstance {
        name: String,
        duration: i32,
        callback: DatumRef,
        target: DatumRef,
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
    XmlRef(u32),
    DateRef(u32),
    MathRef(u32),
    Vector([f64; 3]),
    Null,
}

impl DatumType {
    pub fn type_str(&self) -> String {
        match self {
            DatumType::Int => "int".to_string(),
            DatumType::Float => "float".to_string(),
            DatumType::String => "string".to_string(),
            DatumType::StringChunk => "string_chunk".to_string(),
            DatumType::Void => "void".to_string(),
            DatumType::VarRef => "var_ref".to_string(),
            DatumType::List => "list".to_string(),
            DatumType::XmlChildNodes => "list".to_string(),
            DatumType::PropList => "prop_list".to_string(),
            DatumType::Symbol => "symbol".to_string(),
            DatumType::CastLibRef => "cast_lib".to_string(),
            DatumType::StageRef => "stage".to_string(),
            DatumType::ScriptRef => "script_ref".to_string(),
            DatumType::ScriptInstanceRef => "script_instance".to_string(),
            DatumType::CastMemberRef => "cast_member".to_string(),
            DatumType::SpriteRef => "sprite_ref".to_string(),
            DatumType::Null => "null".to_string(),
            DatumType::ArgList => "arg_list".to_string(),
            DatumType::ArgListNoRet => "arg_list_no_ret".to_string(),
            DatumType::Eval => "eval".to_string(),
            DatumType::Rect => "rect".to_string(),
            DatumType::Point => "point".to_string(),
            DatumType::SoundChannel => "sound_channel".to_string(),
            DatumType::SoundRef => "sound".to_string(),
            DatumType::CursorRef => "cursor_ref".to_string(),
            DatumType::TimeoutRef => "timeout".to_string(),
            DatumType::TimeoutFactory => "timeout_factory".to_string(),
            DatumType::TimeoutInstance => "timeout_instance".to_string(),
            DatumType::ColorRef => "color_ref".to_string(),
            DatumType::BitmapRef => "bitmap_ref".to_string(),
            DatumType::PaletteRef => "palette_ref".to_string(),
            DatumType::Xtra => "xtra".to_string(),
            DatumType::XtraInstance => "xtra_instance".to_string(),
            DatumType::Matte => "matte".to_string(),
            DatumType::PlayerRef => "player_ref".to_string(),
            DatumType::MovieRef => "movie_ref".to_string(),
            DatumType::XmlRef => "xml".to_string(),
            DatumType::DateRef => "date".to_string(),
            DatumType::MathRef => "math".to_string(),
            DatumType::Vector => "vector".to_string(),
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
            Datum::Rect(_) => DatumType::Rect,
            Datum::Point(_) => DatumType::Point,
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
            Datum::XmlRef(_) => DatumType::XmlRef,
            Datum::DateRef(_) => DatumType::DateRef,
            Datum::MathRef(_) => DatumType::MathRef,
            Datum::Vector(_) => DatumType::Vector,
            Datum::Null => DatumType::Null,
        }
    }

    pub fn type_str(&self) -> String {
        self.type_enum().type_str()
    }

    pub fn string_value(&self) -> Result<String, ScriptError> {
        match self {
            Datum::String(s) => Ok(s.clone()),
            Datum::StringChunk(_, _, str_value) => Ok(str_value.to_owned()),
            Datum::Int(n) => Ok(n.to_string()),
            Datum::Symbol(s) => Ok(s.clone()),
            Datum::Vector(v) => Ok(format!("[{},{},{}]", v[0], v[1], v[2])),
            Datum::Rect(r) => Ok(format!("({}, {}, {}, {})", r[0], r[1], r[2], r[3])),
            Datum::ColorRef(cr) => Ok(format!("{:?}", cr)),
            Datum::Void => Ok("VOID".to_string()),
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

    pub fn to_list(&self) -> Result<&Vec<DatumRef>, ScriptError> {
        match self {
            Datum::List(_, items, _) => Ok(items),
            _ => Err(ScriptError::new("Cannot convert datum to list".to_string())),
        }
    }

    pub fn to_list_tuple(&self) -> Result<(&DatumType, &Vec<DatumRef>, bool), ScriptError> {
        match self {
            Datum::List(t, items, sorted) => Ok((t, items, *sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to list".to_string())),
        }
    }

    pub fn to_list_mut(
        &mut self,
    ) -> Result<(&mut DatumType, &mut Vec<DatumRef>, &mut bool), ScriptError> {
        match self {
            Datum::List(t, items, sorted) => Ok((t, items, sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to list".to_string())),
        }
    }

    #[allow(dead_code)]
    pub fn to_map(&self) -> Result<&Vec<(DatumRef, DatumRef)>, ScriptError> {
        match self {
            Datum::PropList(items, ..) => Ok(items),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_map_tuple(&self) -> Result<(&Vec<(DatumRef, DatumRef)>, bool), ScriptError> {
        match self {
            Datum::PropList(items, sorted) => Ok((items, *sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_map_mut(&mut self) -> Result<&mut Vec<(DatumRef, DatumRef)>, ScriptError> {
        match self {
            Datum::PropList(items, ..) => Ok(items),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_map_tuple_mut(
        &mut self,
    ) -> Result<(&mut Vec<(DatumRef, DatumRef)>, &mut bool), ScriptError> {
        match self {
            Datum::PropList(items, sorted) => Ok((items, sorted)),
            _ => Err(ScriptError::new("Cannot convert datum to map".to_string())),
        }
    }

    pub fn to_rect(&self) -> Result<[DatumRef; 4], ScriptError> {
        match self {
            Datum::Rect(rect) => Ok(rect.clone()),
            _ => Err(ScriptError::new(
                "Cannot convert datum to rect".to_string(),
            )),
        }
    }

    pub fn to_rect_mut(&mut self) -> Result<&mut [DatumRef; 4], ScriptError> {
        match self {
            Datum::Rect(rect) => Ok(rect),
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

    pub fn to_point(&self) -> Result<[DatumRef; 2], ScriptError> {
        match self {
            Datum::Point(point) => Ok(point.clone()),
            _ => Err(ScriptError::new(
                "Cannot convert datum to point".to_string(),
            )),
        }
    }

    pub fn to_point_mut(&mut self) -> Result<&mut [DatumRef; 2], ScriptError> {
        match self {
            Datum::Point(point) => Ok(point),
            _ => Err(ScriptError::new(
                "Cannot convert datum to point".to_string(),
            )),
        }
    }

    pub fn to_bitmap_ref(&self) -> Result<&BitmapRef, ScriptError> {
        match self {
            Datum::BitmapRef(bitmap_ref) => Ok(bitmap_ref),
            _ => Err(ScriptError::new(
                "Cannot convert datum to bitmap ref".to_string(),
            )),
        }
    }

    pub fn to_xtra_instance(&self) -> Result<(&String, &XtraInstanceId), ScriptError> {
        match self {
            Datum::XtraInstance(xtra_name, xtra_instance) => Ok((xtra_name, xtra_instance)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to xtra instance".to_string(),
            )),
        }
    }

    pub fn to_xtra_name(&self) -> Result<&String, ScriptError> {
        match self {
            Datum::Xtra(name) => Ok(name),
            _ => Err(ScriptError::new(
                "Cannot convert datum to xtra name".to_string(),
            )),
        }
    }

    pub fn to_string_chunk(
        &self,
    ) -> Result<(&StringChunkSource, &StringChunkExpr, &String), ScriptError> {
        match self {
            Datum::StringChunk(datum_ref, expr, value) => Ok((datum_ref, expr, value)),
            _ => Err(ScriptError::new(
                "Cannot convert datum to string chunk".to_string(),
            )),
        }
    }

    pub fn to_string_mut(&mut self) -> Result<&mut String, ScriptError> {
        match self {
            Datum::String(s) => Ok(s),
            _ => Err(ScriptError::new(
                "Cannot convert datum to string".to_string(),
            )),
        }
    }

    pub fn to_mask(&self) -> Result<&BitmapMask, ScriptError> {
        match self {
            Datum::Matte(mask) => Ok(mask),
            _ => Err(ScriptError::new("Cannot convert datum to mask".to_string())),
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
