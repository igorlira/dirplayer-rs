use core::fmt;
use std::fmt::Formatter;

use log::warn;

use super::{
    bitmap::{
        bitmap::{decode_jpeg_bitmap, decompress_bitmap, Bitmap, BuiltInPalette, PaletteRef},
        manager::{BitmapManager, BitmapRef},
    },
    score::Score,
    sprite::ColorRef,
    ScriptError,
};
use crate::director::chunks::sound::SoundChunk;
use crate::director::chunks::Chunk;
use crate::director::enums::SoundInfo;
use crate::director::{
    chunks::{cast_member::CastMemberDef, score::ScoreChunk},
    enums::{
        BitmapInfo, FilmLoopInfo, FontInfo, MemberType, ScriptType, ShapeInfo, TextMemberData,
    },
    lingo::script::ScriptContext,
};
use crate::player::handlers::datum_handlers::cast_member::font::StyledSpan;
use web_sys::console;

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
    pub back_color: u16,
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
    pub html_styled_spans: Vec<StyledSpan>,
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
            back_color: 0,
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
            html_styled_spans: Vec::new(),
        }
    }

    pub fn has_html_styling(&self) -> bool {
        !self.html_styled_spans.is_empty()
    }

    pub fn get_text_content(&self) -> &str {
        if self.has_html_styling() {
            // Extract plain text from HTML spans
            &self.text
        } else {
            &self.text
        }
    }
}

#[derive(Clone)]
pub struct ScriptMember {
    pub script_id: u32,
    pub script_type: ScriptType,
    pub name: String,
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
    pub shape_info: ShapeInfo,
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
    pub preview_font_name: Option<String>,
    pub preview_html_spans: Vec<StyledSpan>,
    pub fixed_line_space: u16,
    pub top_spacing: i16,
    pub bitmap_ref: Option<BitmapRef>,
    pub char_width: Option<u16>,
    pub char_height: Option<u16>,
    pub grid_columns: Option<u8>,
    pub grid_rows: Option<u8>,
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
    Unknown,
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
    Unknown,
}

impl fmt::Debug for CastMemberType {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field(_) => {
                write!(f, "Field")
            }
            Self::Text(_) => {
                write!(f, "Text")
            }
            Self::Script(_) => {
                write!(f, "Script")
            }
            Self::Bitmap(_) => {
                write!(f, "Bitmap")
            }
            Self::Palette(_) => {
                write!(f, "Palette")
            }
            Self::Shape(_) => {
                write!(f, "Shape")
            }
            Self::FilmLoop(_) => {
                write!(f, "FilmLoop")
            }
            Self::Sound(_) => {
                write!(f, "Sound")
            }
            Self::Font(_) => {
                write!(f, "Font")
            }
            Self::Unknown => {
                write!(f, "Unknown")
            }
        }
    }
}

impl CastMemberTypeId {
    pub fn symbol_string(&self) -> Result<&str, ScriptError> {
        return match self {
            Self::Field => Ok("field"),
            Self::Text => Ok("text"),
            Self::Script => Ok("script"),
            Self::Bitmap => Ok("bitmap"),
            Self::Palette => Ok("palette"),
            Self::Shape => Ok("shape"),
            Self::FilmLoop => Ok("filmLoop"),
            Self::Sound => Ok("sound"),
            Self::Font => Ok("Font"),
            _ => Err(ScriptError::new("Unknown cast member type".to_string())),
        };
    }
}

impl CastMemberType {
    pub fn member_type_id(&self) -> CastMemberTypeId {
        return match self {
            Self::Field(_) => CastMemberTypeId::Field,
            Self::Text(_) => CastMemberTypeId::Text,
            Self::Script(_) => CastMemberTypeId::Script,
            Self::Bitmap(_) => CastMemberTypeId::Bitmap,
            Self::Palette(_) => CastMemberTypeId::Palette,
            Self::Shape(_) => CastMemberTypeId::Shape,
            Self::FilmLoop(_) => CastMemberTypeId::FilmLoop,
            Self::Sound(_) => CastMemberTypeId::Sound,
            Self::Font(_) => CastMemberTypeId::Font,
            Self::Unknown => CastMemberTypeId::Unknown,
        };
    }

    pub fn type_string(&self) -> &str {
        return match self {
            Self::Field(_) => "field",
            Self::Text(_) => "text",
            Self::Script(_) => "script",
            Self::Bitmap(_) => "bitmap",
            Self::Palette(_) => "palette",
            Self::Shape(_) => "shape",
            Self::FilmLoop(_) => "filmLoop",
            Self::Sound(_) => "sound",
            Self::Font(_) => "font",
            _ => "unknown",
        };
    }

    #[allow(dead_code)]
    pub fn as_script(&self) -> Option<&ScriptMember> {
        return match self {
            Self::Script(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_field(&self) -> Option<&FieldMember> {
        return match self {
            Self::Field(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_field_mut(&mut self) -> Option<&mut FieldMember> {
        return match self {
            Self::Field(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_text(&self) -> Option<&TextMember> {
        return match self {
            Self::Text(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_text_mut(&mut self) -> Option<&mut TextMember> {
        return match self {
            Self::Text(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_bitmap(&self) -> Option<&BitmapMember> {
        return match self {
            Self::Bitmap(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_bitmap_mut(&mut self) -> Option<&mut BitmapMember> {
        return match self {
            Self::Bitmap(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_palette(&self) -> Option<&PaletteMember> {
        return match self {
            Self::Palette(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_film_loop(&self) -> Option<&FilmLoopMember> {
        return match self {
            Self::FilmLoop(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_film_loop_mut(&mut self) -> Option<&mut FilmLoopMember> {
        return match self {
            Self::FilmLoop(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_sound(&self) -> Option<&SoundMember> {
        return match self {
            Self::Sound(data) => Some(data),
            _ => None,
        };
    }

    pub fn as_font(&self) -> Option<&FontMember> {
        return match self {
            Self::Font(data) => Some(data),
            _ => None,
        };
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
            Chunk::XMedia(_) => "XMedia",
            Chunk::CstInfo(_) => "Cinf",
            Chunk::Effect(_) => "FXmp",
            Chunk::Thum(_) => "Thum",
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
        def.children.iter().any(|c| match c {
            Some(Chunk::Sound(_)) => true,
            Some(Chunk::Media(m)) => !m.audio_data.is_empty(),
            Some(Chunk::CastMember(_)) => false,
            _ => false,
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
        let abmp_chunk = member_def.children.get(0).and_then(|x| x.as_ref());

        if let Some(abmp_chunk) = abmp_chunk {
            let abmp_chunk = abmp_chunk.as_bitmap().unwrap();
            let decompressed =
                decompress_bitmap(&abmp_chunk.data, &bitmap_info, cast_lib, abmp_chunk.version);
            match decompressed {
                Ok(new_bitmap) => bitmap_manager.add_bitmap(new_bitmap),
                Err(e) => {
                    warn!(
                        "Failed to decompress bitmap {}: {:?}. Using empty image.",
                        number, e
                    );
                    bitmap_manager.add_bitmap(Bitmap::new(
                        1,
                        1,
                        8,
                        8,
                        0,
                        PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
                    ))
                }
            }
        } else {
            warn!("No bitmap chunk found for member {}", number);
            bitmap_manager.add_bitmap(Bitmap::new(
                1,
                1,
                8,
                8,
                0,
                PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
            ))
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
                let text_chunk = member_def.children[0]
                    .as_ref()
                    .unwrap()
                    .as_text()
                    .expect("Not a text chunk");
                let mut field_member = FieldMember::new();
                field_member.text = text_chunk.text.clone();
                CastMemberType::Field(field_member)
            }
            MemberType::Script => {
                let member_info = chunk.member_info.as_ref().unwrap();
                let script_id = member_info.header.script_id;
                let script_type = chunk.specific_data.script_type().unwrap();
                let _script_chunk = &lctx.as_ref().unwrap().scripts[&script_id];

                CastMemberType::Script(ScriptMember {
                    script_id,
                    script_type,
                    name: member_info.name.clone(),
                })
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

                // Step 1: Extract font name from multiple sources
                let font_name = {
                    // Try member_info first
                    if let Some(info) = &chunk.member_info {
                        if !info.name.is_empty() {
                            info.name.clone()
                        } else {
                            // Try to extract from PFR font data
                            member_def
                                .children
                                .iter()
                                .find_map(|child_opt| {
                                    if let Some(Chunk::XMedia(xmedia)) = child_opt {
                                        if xmedia.is_pfr_font() {
                                            return xmedia.extract_font_name();
                                        }
                                    }
                                    None
                                })
                                .unwrap_or_else(|| {
                                    // Try to extract from FontInfo in specific_data
                                    if let Some(info) = chunk.specific_data.font_info() {
                                        if !info.name.is_empty() {
                                            return info.name.clone();
                                        }
                                    }
                                    // Last resort: generic name
                                    format!("Font_{}", number)
                                })
                        }
                    } else {
                        // No member_info, try PFR
                        member_def
                            .children
                            .iter()
                            .find_map(|child_opt| {
                                if let Some(Chunk::XMedia(xmedia)) = child_opt {
                                    if xmedia.is_pfr_font() {
                                        return xmedia.extract_font_name();
                                    }
                                }
                                None
                            })
                            .unwrap_or_else(|| format!("Font_{}", number))
                    }
                };

                console::log_1(
                    &format!(
                        "🎨 Creating Font cast member: '{}' (member #{})",
                        font_name, number
                    )
                    .into(),
                );

                let specific_data_bytes = chunk.specific_data_raw.clone();

                // Step 2: Extract preview text from XMedia (the styled text one)
                let preview_text = member_def
                    .children
                    .iter()
                    .find_map(|child_opt| {
                        if let Some(Chunk::XMedia(xmedia)) = child_opt {
                            if !xmedia.is_pfr_font() && xmedia.raw_data.len() > 100 {
                                console::log_1(
                                    &format!(
                                        "📄 Analyzing text XMedia chunk ({} bytes)",
                                        xmedia.raw_data.len()
                                    )
                                    .into(),
                                );

                                // Debug: show first 200 bytes
                                let preview: String = xmedia
                                    .raw_data
                                    .iter()
                                    .take(200)
                                    .map(|b| format!("{:02X}", b))
                                    .collect::<Vec<_>>()
                                    .join(" ");
                                console::log_1(&format!("   First 200 bytes: {}", preview).into());

                                // Look for the pattern "00 XX XX 2C" followed by readable text
                                for i in 0..xmedia.raw_data.len().saturating_sub(100) {
                                    if xmedia.raw_data[i] == 0x00
                                        && i + 3 < xmedia.raw_data.len()
                                        && xmedia.raw_data[i + 3] == 0x2C
                                    {
                                        // comma

                                        console::log_1(
                                            &format!(
                                                "   Found potential text start at offset {}: {:02X} {:02X} {:02X} {:02X}",
                                                i,
                                                xmedia.raw_data[i],
                                                xmedia.raw_data[i+1],
                                                xmedia.raw_data[i+2],
                                                xmedia.raw_data[i+3]
                                            )
                                            .into(),
                                        );

                                        // Parse the length from the hex ASCII digits before the comma
                                        // Pattern is: 00 XX XX 2C where XX XX are ASCII hex digits
                                        let mut length_str = String::new();
                                        let mut length_pos = i + 1;
                                        
                                        while length_pos < i + 3 && xmedia.raw_data[length_pos] != 0x2C {
                                            let byte = xmedia.raw_data[length_pos];
                                            if byte >= 0x30 && byte <= 0x39 { // 0-9
                                                length_str.push(byte as char);
                                            } else if byte >= 0x41 && byte <= 0x46 { // A-F
                                                length_str.push(byte as char);
                                            } else if byte >= 0x61 && byte <= 0x66 { // a-f
                                                length_str.push(byte as char);
                                            }
                                            length_pos += 1;
                                        }
                                        
                                        console::log_1(
                                            &format!("   Length string: '{}'", length_str).into(),
                                        );

                                        // Parse the length (it's in hex format as ASCII)
                                        let text_length = if let Ok(len) = usize::from_str_radix(&length_str, 16) {
                                            console::log_1(
                                                &format!("   Parsed length: {} bytes (0x{} hex)", len, length_str).into(),
                                            );
                                            len
                                        } else {
                                            console::log_1(&"   Failed to parse length, using fallback".into());
                                            100 // fallback
                                        };

                                        // Start reading after the comma
                                        let text_start = i + 4;
                                        let mut text = String::new();

                                        for j in text_start..xmedia.raw_data.len().min(text_start + text_length) {
                                            let byte = xmedia.raw_data[j];

                                            if byte >= 0x20 && byte <= 0x7E {
                                                text.push(byte as char);
                                            } else if byte == 0x0D || byte == 0x0A {
                                                text.push(' ');
                                            }
                                        }

                                        // Clean up the text
                                        let text = text.trim().to_string();

                                        console::log_1(
                                            &format!(
                                                "   Extracted: '{}' ({} chars)",
                                                text.chars().take(80).collect::<String>(),
                                                text.len()
                                            )
                                            .into(),
                                        );

                                        if text.len() > 3 {
                                            return Some(text);
                                        }
                                    }
                                }

                                console::log_1(&"   ⚠️ No text found in XMedia chunk".into());
                            }
                        }
                        None
                    })
                    .unwrap_or_default();

                console::log_1(
                    &format!("   Final preview_text length: {}", preview_text.len()).into(),
                );

                let preview_font_name = member_def.children.iter().find_map(|child_opt| {
                    if let Some(Chunk::XMedia(xmedia)) = child_opt {
                        if !xmedia.is_pfr_font() && xmedia.raw_data.len() > 100 {
                            // Look for font names in the styled text data
                            // Fonts are usually referenced as null-terminated strings
                            for i in 0..xmedia.raw_data.len().saturating_sub(20) {
                                if xmedia.raw_data[i..].starts_with(b"FFF Reaction") {
                                    let mut name = Vec::new();
                                    for &b in &xmedia.raw_data[i..] {
                                        if b == 0 {
                                            break;
                                        }
                                        if b >= 0x20 && b <= 0x7E {
                                            name.push(b);
                                        }
                                    }
                                    if !name.is_empty() {
                                        let name_str = String::from_utf8_lossy(&name).to_string();
                                        console::log_1(
                                            &format!("📝 Found font reference: '{}'", name_str)
                                                .into(),
                                        );
                                        return Some(name_str);
                                    }
                                }
                            }
                        }
                    }
                    None
                });

                // Step 3: Parse PFR font data from XMedia
                let pfr_font_data = member_def.children.iter().find_map(|child_opt| {
                    if let Some(Chunk::XMedia(xmedia)) = child_opt {
                        if xmedia.is_pfr_font() {
                            console::log_1(
                                &format!(
                                    "🎨 Found PFR font data ({} bytes)",
                                    xmedia.raw_data.len()
                                )
                                .into(),
                            );
                            return xmedia.parse_pfr_font();
                        }
                    }
                    None
                });

                // Step 4: Determine what kind of font member this is
                let is_text_data = FontInfo::looks_like_text_data(&specific_data_bytes);
                let has_font_binary = FontInfo::looks_like_real_font_data(&specific_data_bytes);

                // Step 5: Create the appropriate FontMember
                let (font_info, bitmap_ref, char_width, char_height, grid_columns, grid_rows) =
                    if let Some(pfr) = pfr_font_data {
                        console::log_1(
                            &format!(
                                "📝 Creating bitmap from PFR: '{}' ({}x{}, grid {}x{})",
                                pfr.font_name,
                                pfr.char_width,
                                pfr.char_height,
                                pfr.grid_columns,
                                pfr.grid_rows
                            )
                            .into(),
                        );

                        let bitmap_width = pfr.char_width * pfr.grid_columns as u16;
                        let bitmap_height = pfr.char_height * pfr.grid_rows as u16;

                        let mut font_bitmap = Bitmap::new(
                            bitmap_width,
                            bitmap_height,
                            32,
                            32,
                            0,
                            PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
                        );

                        // Clear to transparent
                        for pixel in font_bitmap.data.chunks_exact_mut(4) {
                            pixel[0] = 0;
                            pixel[1] = 0;
                            pixel[2] = 0;
                            pixel[3] = 0;
                        }

                        // ✅ RENDER VECTOR GLYPHS
                        console::log_1(&"🎨 Rendering PFR vector glyphs...".into());

                        use super::super::director::chunks::pfr_renderer::render_pfr_font;

                        let rendered_bitmap_data = render_pfr_font(
                            &pfr.glyph_data,
                            pfr.char_width as usize,
                            pfr.char_height as usize,
                            128, // Total glyphs
                        );

                        console::log_1(
                            &format!(
                                "✅ Vector rendering complete: {} bytes",
                                rendered_bitmap_data.len()
                            )
                            .into(),
                        );

                        // Copy the rendered 1-bit data into the 32-bit RGBA bitmap
                        let bytes_per_row = ((pfr.char_width + 7) / 8) as usize;
                        let bytes_per_glyph = bytes_per_row * pfr.char_height as usize;

                        for glyph_idx in 0..128 {
                            let grid_x = (glyph_idx % pfr.grid_columns as usize) as u16;
                            let grid_y = (glyph_idx / pfr.grid_columns as usize) as u16;
                            let dest_x = grid_x * pfr.char_width;
                            let dest_y = grid_y * pfr.char_height;

                            let glyph_offset = glyph_idx * bytes_per_glyph;

                            // Decode each row of the glyph
                            for row in 0..pfr.char_height {
                                let row_offset = glyph_offset + (row as usize * bytes_per_row);

                                if row_offset >= rendered_bitmap_data.len() {
                                    break;
                                }

                                // Process each pixel in the row
                                for col in 0..pfr.char_width {
                                    let byte_idx = row_offset + (col as usize / 8);

                                    if byte_idx >= rendered_bitmap_data.len() {
                                        continue;
                                    }

                                    let byte = rendered_bitmap_data[byte_idx];
                                    let bit_idx = 7 - (col % 8);
                                    let bit_set = (byte >> bit_idx) & 1 == 1;

                                    if bit_set {
                                        let px = dest_x + col;
                                        let py = dest_y + row;

                                        if px < bitmap_width && py < bitmap_height {
                                            let pixel_idx = (py as usize * bitmap_width as usize
                                                + px as usize)
                                                * 4;
                                            if pixel_idx + 3 < font_bitmap.data.len() {
                                                font_bitmap.data[pixel_idx] = 255; // R
                                                font_bitmap.data[pixel_idx + 1] = 255; // G
                                                font_bitmap.data[pixel_idx + 2] = 255; // B
                                                font_bitmap.data[pixel_idx + 3] = 255;
                                                // A
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        console::log_1(&"✅ PFR font bitmap created!".into());

                        let bitmap_ref = bitmap_manager.add_bitmap(font_bitmap);

                        let final_font_name = if !pfr.font_name.is_empty() {
                            pfr.font_name.clone()
                        } else {
                            font_name.clone()
                        };

                        let font_info = FontInfo {
                            font_id: 0,
                            size: pfr.char_height,
                            style: 0,
                            name: final_font_name,
                        };

                        console::log_1(&"✅ PFR font bitmap created!".into());

                        (
                            font_info,
                            Some(bitmap_ref),
                            Some(pfr.char_width),
                            Some(pfr.char_height),
                            Some(pfr.grid_columns),
                            Some(pfr.grid_rows),
                        )
                    } else if has_font_binary {
                        // Has font binary but no PFR - use FontInfo
                        let font_info = if let Some(info) = chunk.specific_data.font_info() {
                            let mut info = info.clone();
                            if info.name.is_empty() {
                                info.name = font_name.clone();
                            }
                            info
                        } else {
                            FontInfo {
                                font_id: 0,
                                size: 12,
                                style: 0,
                                name: font_name.clone(),
                            }
                        };

                        (font_info, None, None, None, None, None)
                    } else {
                        // Text data only - create minimal font info
                        let font_info = FontInfo {
                            font_id: 0,
                            size: 12,
                            style: 0,
                            name: font_name.clone(),
                        };
                        (font_info, None, None, None, None, None)
                    };

                // Step 6: Create the single FontMember
                console::log_1(
                    &format!(
                        "✅ Creating FontMember: name='{}', has_bitmap={}, preview_text_len={}",
                        font_info.name,
                        bitmap_ref.is_some(),
                        preview_text.len()
                    )
                    .into(),
                );

                CastMemberType::Font(FontMember {
                    font_info,
                    preview_text,
                    preview_font_name,
                    preview_html_spans: Vec::new(),
                    fixed_line_space: 14,
                    top_spacing: 0,
                    bitmap_ref,
                    char_width,
                    char_height,
                    grid_columns,
                    grid_rows,
                })
            }
            MemberType::Bitmap => {
                let bitmap_info = chunk.specific_data.bitmap_info().unwrap();

                // First, check if there's a Media (ediM) chunk with JPEG data
                let media_chunk = member_def.children.iter().find_map(|c| {
                    c.as_ref().and_then(|chunk| match chunk {
                        Chunk::Media(m) => Some(m),
                        _ => None,
                    })
                });

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
                        web_sys::console::log_1(
                            &format!(
                                "Found JPEG data in Media chunk for bitmap {}, size: {} bytes",
                                number,
                                media.audio_data.len()
                            )
                            .into(),
                        );

                        match decode_jpeg_bitmap(&media.audio_data, &bitmap_info) {
                            Ok(new_bitmap) => {
                                web_sys::console::log_1(
                                    &format!(
                                        "Successfully decoded JPEG: {}x{}, bit_depth: {}",
                                        new_bitmap.width, new_bitmap.height, new_bitmap.bit_depth
                                    )
                                    .into(),
                                );
                                bitmap_manager.add_bitmap(new_bitmap)
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to decode JPEG bitmap {}: {:?}. Using empty image.",
                                    number, e
                                );
                                bitmap_manager.add_bitmap(Bitmap::new(
                                    1,
                                    1,
                                    8,
                                    8,
                                    0,
                                    PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
                                ))
                            }
                        }
                    } else {
                        // Media chunk exists but doesn't contain JPEG, fall back to BITD
                        Self::decode_bitmap_from_bitd(
                            member_def,
                            &bitmap_info,
                            cast_lib,
                            number,
                            bitmap_manager,
                        )
                    }
                } else {
                    // No Media chunk, use BITD
                    Self::decode_bitmap_from_bitd(
                        member_def,
                        &bitmap_info,
                        cast_lib,
                        number,
                        bitmap_manager,
                    )
                };

                CastMemberType::Bitmap(BitmapMember {
                    image_ref: new_bitmap_ref,
                    reg_point: (bitmap_info.reg_x, bitmap_info.reg_y),
                })
            }
            MemberType::Palette => {
                let palette_chunk = member_def.children[0]
                    .as_ref()
                    .unwrap()
                    .as_palette()
                    .expect("Not a palette chunk");
                CastMemberType::Palette(PaletteMember {
                    colors: palette_chunk.colors.clone(),
                })
            }
            MemberType::Shape => CastMemberType::Shape(ShapeMember {
                shape_info: chunk.specific_data.shape_info().unwrap().clone(),
            }),
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
                    console::log_1(
                        &format!(
                            "CastMember {} has {} children:",
                            number,
                            member_def.children.len()
                        )
                        .into(),
                    );

                    for (i, c_opt) in member_def.children.iter().enumerate() {
                        match c_opt {
                            Some(c) => console::log_1(
                                &format!("child[{}] = {}", i, Self::chunk_type_name(c)).into(),
                            ),
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
                console::log_1(
                    &format!(
                        "CastMember {}: {} children, found sound chunk = {}",
                        number,
                        member_def.children.len(),
                        found_sound
                    )
                    .into(),
                );

                // Construct SoundMember
                if let Some(sound_chunk) = sound_chunk_opt {
                    let info = SoundInfo {
                        sample_rate: sound_chunk.sample_rate(),
                        sample_size: sound_chunk.bits_per_sample(),
                        channels: sound_chunk.channels(),
                        sample_count: sound_chunk.sample_count(),
                        duration: if sound_chunk.sample_rate() > 0 {
                            (sound_chunk.sample_count() as f32 / sound_chunk.sample_rate() as f32
                                * 1000.0)
                                .round() as u32
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
                // Assuming `chunk.member_type` is an enum backed by a numeric ID
                // If it's not Copy, clone or cast as needed.
                let member_type_id = chunk.member_type as u16; // or u32 depending on your enum base type

                console::log_1(
                    &format!(
          "[CastMember::from] Unknown member type for member #{} (cast_lib={}): {:?} (id={})",
          number,
          cast_lib,
          chunk.member_type, // this prints name, e.g. Button
          member_type_id      // this prints numeric id, e.g. 15
        )
                    .into(),
                );

                if let Some(info) = &chunk.member_info {
                    console::log_1(
                        &format!(
                            "  → name='{}', script_id={}, flags={:?}",
                            info.name, info.header.script_id, info.header.flags
                        )
                        .into(),
                    );
                } else {
                    console::log_1(&"  → No member_info available".into());
                }

                // Log all child chunks
                if member_def.children.is_empty() {
                    console::log_1(&"  → No children found.".into());
                } else {
                    console::log_1(&format!("  → {} children:", member_def.children.len()).into());

                    for (i, c_opt) in member_def.children.iter().enumerate() {
                        match c_opt {
                            Some(c) => console::log_1(
                                &format!("    child[{}] = {}", i, Self::chunk_type_name(c)).into(),
                            ),
                            None => console::log_1(&format!("    child[{}] = None", i).into()),
                        }
                    }
                }

                CastMemberType::Unknown
            }
        };
        CastMember {
            number,
            name: chunk
                .member_info
                .as_ref()
                .map(|x| x.name.to_owned())
                .unwrap_or_default(),
            member_type: member_type,
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }
}
