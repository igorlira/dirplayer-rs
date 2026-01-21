use core::fmt;
use std::fmt::Formatter;

use log::{debug, warn};

use crate::CastMemberRef;

use super::{
    bitmap::{
        bitmap::{decode_jpeg_bitmap, decompress_bitmap, Bitmap, BuiltInPalette, PaletteRef},
        manager::{BitmapManager, BitmapRef},
    },
    score::Score,
    sprite::ColorRef,
    ScriptError,
};
use crate::director::{
    chunks::{cast_member::CastMemberDef, score::ScoreChunk, xmedia::PfrFont, xmedia::XMediaChunk, sound::SoundChunk, Chunk, cast_member::CastMemberChunk},
    enums::{
        BitmapInfo, FilmLoopInfo, FontInfo, MemberType, ScriptType, ShapeInfo, TextMemberData, SoundInfo, FieldInfo,
    },
    lingo::script::ScriptContext,
};
use crate::player::handlers::datum_handlers::cast_member::font::{StyledSpan, TextAlignment};

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

pub struct PfrBitmap {
    pub bitmap_ref: BitmapRef,
    pub char_width: u16,
    pub char_height: u16,
    pub grid_columns: u8,
    pub grid_rows: u8,
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

    pub fn from_field_info(field_info: FieldInfo) -> FieldMember {
        FieldMember {
            text: "".to_string(),
            alignment: field_info.alignment_str(),
            word_wrap: field_info.wordwrap(),
            font: field_info.font_name().to_string(),
            font_style: "plain".to_string(),
            font_size: 12,
            fixed_line_space: field_info.height as u16,
            top_spacing: field_info.scroll_top as i16,
            box_type: field_info.box_type_str(),
            anti_alias: false,
            width: field_info.width as u16,
            auto_tab: field_info.auto_tab(),
            editable: field_info.editable(),
            border: field_info.border as u16,
            back_color: field_info.bg_color(),
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
    pub script_id: u32,
    pub member_script_ref: Option<CastMemberRef>,
    pub info: BitmapInfo,
}

#[derive(Clone, Debug)]
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

#[derive(Clone)]
pub struct FlashMember {
    pub data: Vec<u8>,
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
    pub alignment: TextAlignment,
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
    Flash(FlashMember),
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
    Flash,
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
            Self::Flash(_) => {
                write!(f, "Flash")
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
            Self::Font => Ok("font"),
            Self::Flash => Ok("flash"),
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
            Self::Flash(_) => CastMemberTypeId::Flash,
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
            Self::Flash(_) => "flash",
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

    pub fn as_flash(&self) -> Option<&FlashMember> {
        return match self {
            Self::Flash(data) => { Some(data) }
            _ => { None }
        }
    }

    pub fn as_flash_mut(&mut self) -> Option<&mut FlashMember> {
        return match self {
            Self::Flash(data) => { Some(data) }
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
            Chunk::XMedia(_) => "XMedia",
            Chunk::CstInfo(_) => "Cinf",
            Chunk::Effect(_) => "FXmp",
            Chunk::Thum(_) => "Thum",
            Chunk::Raw(_) => "Raw",
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

    fn extract_text_from_xmedia(data: &[u8]) -> Option<String> {
        let mut i = 0;

        while i < data.len() {
            // find '2C' which marks the start of text
            if data[i] != 0x2C {
                i += 1;
                continue;
            }

            let start = i + 1;

            // find following 03 byte
            let mut end = start;
            while end < data.len() && data[end] != 0x03 {
                end += 1;
            }

            // if 03 not found → no valid text block
            if end >= data.len() {
                return None;
            }

            // extract text bytes
            let raw = &data[start..end];
            let mut text = String::new();

            for &b in raw {
                match b {
                    0x20..=0x7E => text.push(b as char), // printable ASCII
                    0x09 => text.push('\t'),             // preserve TAB
                    0x0D => text.push('\r'),             // preserve CR
                    0x0A => text.push('\n'),             // preserve LF
                    _ => {}                              // skip weird bytes
                }
            }

            let cleaned = text.trim().to_string();
            if !cleaned.is_empty() {
                return Some(cleaned);
            }

            i = end + 1;
        }

        None
    }

    fn scan_font_name_from_xmedia(xmedia: &XMediaChunk) -> Option<String> {
        let data = &xmedia.raw_data;

        for i in 0..data.len().saturating_sub(20) {
            // Look for the exact prefix
            if data[i..].starts_with(b"FFF Reaction") {
                // Extract until the null terminator
                let mut name = Vec::new();

                for &b in &data[i..] {
                    if b == 0 { break; }
                    if b.is_ascii_graphic() || b == b' ' {
                        name.push(b);
                    }
                }

                if !name.is_empty() {
                    return Some(String::from_utf8_lossy(&name).to_string());
                }
            }
        }

        None
    }

    fn extract_pfr(member_def: &CastMemberDef) -> Option<PfrFont> {
        member_def.children.iter()
            .find_map(|c| match c {
                Some(Chunk::XMedia(x)) if x.is_pfr_font() => x.parse_pfr_font(),
                _ => None
            })
    }

    fn find_pfr_name(member_def: &CastMemberDef) -> Option<String> {
        member_def.children.iter().find_map(|child_opt| {
            if let Some(Chunk::XMedia(xmedia)) = child_opt {
                if xmedia.is_pfr_font() {
                    let name = xmedia.extract_font_name();
                    if !name.clone()?.is_empty() {
                        return Some(name);
                    }
                }
            }
            None
        })?
    }

    fn resolve_font_name(chunk: &CastMemberChunk, member_def: &CastMemberDef, number: u32) -> String {
        if let Some(name) = chunk.member_info.as_ref().map(|i| i.name.clone()).filter(|n| !n.is_empty()) {
            return name;
        }

        if let Some(pfr_name) = Self::find_pfr_name(member_def) {
            return pfr_name;
        }

        if let Some(info) = chunk.specific_data.font_info() {
            if !info.name.is_empty() {
                return info.name.clone();
            }
        }

        format!("Font_{}", number)
    }

    fn render_pfr_to_bitmap(
        pfr: &PfrFont,
        bitmap_manager: &mut BitmapManager,
    ) -> PfrBitmap {
        let bitmap_width  = (pfr.char_width  as u16) * (pfr.grid_columns as u16);
        let bitmap_height = (pfr.char_height as u16) * (pfr.grid_rows    as u16);

        debug!(
            "🎨 Creating bitmap for PFR font '{}' ({}x{}, grid {}x{})",
            pfr.font_name,
            bitmap_width,
            bitmap_height,
            pfr.grid_columns,
            pfr.grid_rows,
        );

        // Create a blank 32-bit bitmap
        let mut bitmap = Bitmap::new(
            bitmap_width,
            bitmap_height,
            32,
            32,
            0,
            PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
        );

        // fully transparent
        for px in bitmap.data.chunks_exact_mut(4) {
            px.copy_from_slice(&[0, 0, 0, 0]);
        }

        // Render PFR vector glyphs into a temporary 1-bit buffer
        use super::super::director::chunks::pfr_renderer::render_pfr_font;

        let rendered = render_pfr_font(
            &pfr.glyph_data,
            pfr.char_width as usize,
            pfr.char_height as usize,
            128, // fixed number of glyphs
        );

        debug!("🖨 Rendered PFR vector glyphs → {} bytes", rendered.len());

        let bytes_per_row = ((pfr.char_width + 7) / 8) as usize;
        let bytes_per_glyph = bytes_per_row * pfr.char_height as usize;

        // Copy the 1-bit glyphs into the RGBA bitmap
        for glyph_idx in 0..128 {
            let grid_x = (glyph_idx % pfr.grid_columns as usize) as u8;
            let grid_y = (glyph_idx / pfr.grid_columns as usize) as u8;

            let dest_x = (grid_x as u16) * (pfr.char_width as u16);
            let dest_y = (grid_y as u16) * (pfr.char_height as u16);

            let glyph_offset = glyph_idx * bytes_per_glyph;

            for row in 0..pfr.char_height {
                let row_base = glyph_offset + (row as usize * bytes_per_row);
                if row_base >= rendered.len() {
                    break;
                }

                for col in 0..pfr.char_width {
                    let b_index = row_base + (col as usize / 8);
                    if b_index >= rendered.len() {
                        continue;
                    }

                    let byte = rendered[b_index];
                    let bit = (byte >> (7 - (col % 8))) & 1;

                    if bit == 1 {
                        let px = dest_x + col as u16;
                        let py = dest_y + row as u16;
                        let index = (py as usize * bitmap_width as usize + px as usize) * 4;

                        bitmap.data[index..index + 4].copy_from_slice(&[255, 255, 255, 255]);
                    }
                }
            }
        }

        debug!("✅ Finished assembling PFR bitmap.");

        // Add to the manager and return
        let bitmap_ref = bitmap_manager.add_bitmap(bitmap);

        PfrBitmap {
            bitmap_ref,
            char_width: pfr.char_width,
            char_height: pfr.char_height,
            grid_columns: pfr.grid_columns,
            grid_rows: pfr.grid_rows,
        }
    }

    fn log_ole_start(number: u32, cast_lib: u32, chunk: &CastMemberChunk) {
        debug!(
            "Processing Ole member #{} in cast lib {} (name: {})",
            number,
            cast_lib,
            chunk.member_info.as_ref().map(|x| x.name.as_str()).unwrap_or("")
        );
    }

    fn log_found_swf(number: u32, sig: &[u8], len: usize) {
        debug!("✅ Found SWF data in Ole member #{} (signature: {:?}, {} bytes)", number, sig, len);
    }

    fn log_found_swf_at_offset(number: u32, sig: &[u8]) {
        debug!("✅ Found SWF signature at offset 12 in Ole member #{}: {:?}", number, sig);
    }

    fn log_unknown_ole(number: u32, chunk: &CastMemberChunk) {
        debug!(
            "Cast member #{} has unimplemented type: Ole (name: {})",
            number,
            chunk.member_info.as_ref().map(|x| x.name.as_str()).unwrap_or("")
        );
    }

    fn make_swf_member(number: u32, chunk: &CastMemberChunk, data: Vec<u8>) -> CastMember {
        CastMember {
            number,
            name: chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
            member_type: CastMemberType::Flash(FlashMember { data }),
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }

    fn get_first_child_bytes(member_def: &CastMemberDef) -> Option<Vec<u8>> {
        if let Some(Some(ch)) = member_def.children.get(0) {
            if let Some(bytes) = ch.as_bytes() {
                return Some(bytes.to_vec());
            }
        }
        None
    }

    fn try_parse_swf(bytes: Vec<u8>, number: u32, chunk: &CastMemberChunk) -> Option<CastMember> {
        if bytes.len() < 3 {
            return None;
        }

        let sig = &bytes[0..3];

        let is_swf = sig == b"FWS" || sig == b"CWS" || sig == b"ZWS";
        if is_swf {
            Self::log_found_swf(number, sig, bytes.len());
            return Some(Self::make_swf_member(number, chunk, bytes));
        }

        // Try offset 12 SWF (OLE wrapped SWF)
        if bytes.len() > 15 {
            let sig2 = &bytes[12..15];
            let is_swf2 = sig2 == b"FWS" || sig2 == b"CWS" || sig2 == b"ZWS";
            if is_swf2 {
                Self::log_found_swf_at_offset(number, sig2);
                return Some(Self::make_swf_member(number, chunk, bytes[12..].to_vec()));
            }
        }

        None
    }

    fn scan_children_for_ole(
        member_def: &CastMemberDef,
        number: u32,
        chunk: &CastMemberChunk,
        bitmap_manager: &mut BitmapManager,
    ) -> Option<CastMember> 
    {
        for opt_child in &member_def.children {
            let Some(Chunk::XMedia(xm)) = opt_child else { continue };

            // 1) If SWF: return SWF
            if let Some(cm) = Self::try_parse_swf(xm.raw_data.to_vec(), number, chunk) {
                return Some(cm);
            }

            // 2) Font logic
            return Some(Self::parse_xmedia_font(member_def, number, chunk, xm, bitmap_manager));
        }
        None
    }

    fn parse_xmedia_font(
        member_def: &CastMemberDef,
        number: u32,
        chunk: &CastMemberChunk,
        xm: &XMediaChunk,
        bitmap_manager: &mut BitmapManager,
    ) -> CastMember {
        let font_name = Self::resolve_font_name(chunk, member_def, number);
        let preview_text = Self::extract_text_from_xmedia(&xm.raw_data)
            .filter(|s| s.len() > 3)
            .unwrap_or_default();
        let preview_font_name = Self::scan_font_name_from_xmedia(xm);
        let pfr = Self::extract_pfr(member_def);

        let info_and_bitmap = Self::build_font_info_and_bitmap(
            pfr,
            chunk,
            &font_name,
            bitmap_manager,
        );

        let (font_info, bitmap_ref, char_w, char_h, gc, gr) = info_and_bitmap;

        CastMember {
            number,
            name: chunk
                .member_info
                .as_ref()
                .map(|x| x.name.to_owned())
                .unwrap_or_default(),
            member_type: CastMemberType::Font(FontMember {
                font_info,
                preview_text,
                preview_font_name,
                preview_html_spans: Vec::new(),
                fixed_line_space: 14,
                top_spacing: 0,
                bitmap_ref,
                char_width: char_w,
                char_height: char_h,
                grid_columns: gc,
                grid_rows: gr,
                alignment: TextAlignment::Left,
            }),
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }

    fn build_font_info_and_bitmap(
        pfr: Option<PfrFont>,
        chunk: &CastMemberChunk,
        default_name: &str,
        bitmap_manager: &mut BitmapManager,
    ) -> (
        FontInfo,
        Option<BitmapRef>,
        Option<u16>, // char width
        Option<u16>, // char height
        Option<u8>, // grid columns
        Option<u8>, // grid rows
        ) {
        let specific_bytes = chunk.specific_data_raw.clone();

        if let Some(pfr) = pfr {
            let bmp = Self::render_pfr_to_bitmap(&pfr, bitmap_manager);

            let info = FontInfo {
                font_id: 0,
                size: pfr.char_height,
                style: 0,
                name: if pfr.font_name.is_empty() {
                    default_name.to_string()
                } else {
                    pfr.font_name.clone()
                }
            };

            return (
                info,
                Some(bmp.bitmap_ref),
                Some(bmp.char_width),
                Some(bmp.char_height),
                Some(bmp.grid_columns),
                Some(bmp.grid_rows),
            );
        }

        if FontInfo::looks_like_real_font_data(&specific_bytes) {
            let info = chunk
                .specific_data
                .font_info()
                .map(|fi| fi.clone().with_default_name(default_name))
                .unwrap_or_else(|| FontInfo::minimal(default_name));

            return (info, None, None, None, None, None);
        }

        // fallback
        (FontInfo::minimal(default_name), None, None, None, None, None)
    }

    pub fn get_script_id(&self) -> Option<u32> {
        match &self.member_type {
            CastMemberType::Bitmap(bitmap) => {
                if bitmap.script_id > 0 {
                    Some(bitmap.script_id)
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    pub fn get_member_script_ref(&self) -> Option<&CastMemberRef> {
        match &self.member_type {
            CastMemberType::Bitmap(bitmap) => bitmap.member_script_ref.as_ref(),
            _ => None,
        }
    }

    pub fn set_member_script_ref(&mut self, script_ref: CastMemberRef) {
        match &mut self.member_type {
            CastMemberType::Bitmap(bitmap) => {
                bitmap.member_script_ref = Some(script_ref);
            }
            _ => {}
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
                let field_info = FieldInfo::from(chunk.specific_data_raw.as_slice());
                let mut field_member = FieldMember::from_field_info(field_info);
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
            MemberType::Flash => {
                use crate::director::enums::ShapeType;
                debug!("Flash member {}: checking for shape_info", number);
                debug!("  specific_data has shape_info: {}", chunk.specific_data.shape_info().is_some());
                
                if let Some(shape_info) = chunk.specific_data.shape_info() {
                    debug!("Flash member {} is a Shape (via shape_info)", number);
                    return CastMember {
                        number,
                        name: chunk
                            .member_info
                            .as_ref()
                            .map(|x| x.name.to_owned())
                            .unwrap_or_default(),
                        member_type: CastMemberType::Shape(ShapeMember {
                            shape_info: shape_info.clone(),
                        }),
                        color: ColorRef::PaletteIndex(255),
                        bg_color: ColorRef::PaletteIndex(0),
                    }
                }
                
                // Director MX 2004 can store shapes as Flash members
                // Try to parse the specific_data_raw as ShapeInfo
                if !chunk.specific_data_raw.is_empty() {
                    debug!("  specific_data_raw length: {}", chunk.specific_data_raw.len());
                    
                    // Try parsing as ShapeInfo
                    let shape_info = ShapeInfo::from(chunk.specific_data_raw.as_slice());
                    debug!("  Parsed shape_type: {:?}", shape_info.shape_type);
                    
                    // If it looks like valid shape data, treat it as a shape
                    if matches!(shape_info.shape_type, ShapeType::Rect | ShapeType::Oval | ShapeType::OvalRect | ShapeType::Line) {
                        debug!("Flash member {} is actually a Shape!", number);
                        return CastMember {
                            number,
                            name: chunk
                                .member_info
                                .as_ref()
                                .map(|x| x.name.to_owned())
                                .unwrap_or_default(),
                            member_type: CastMemberType::Shape(ShapeMember { shape_info }),
                            color: ColorRef::PaletteIndex(255),
                            bg_color: ColorRef::PaletteIndex(0),
                        }
                    }
                }
                
                // Otherwise, process as actual Flash
                debug!("Creating Flash cast member #{} in cast lib {}", number, cast_lib);
                if let Some(Some(chunk)) = member_def.children.get(0) {
                    if let Some(flash_chunk_data) = chunk.as_bytes() {
                        CastMemberType::Flash(FlashMember { data: flash_chunk_data.to_vec() })
                    } else {
                        warn!("Flash cast member data chunk was not of expected type.");
                        CastMemberType::Flash(FlashMember { data: vec![] })
                    }
                } else {
                    warn!("Flash cast member has no data chunk or it is invalid.");
                    CastMemberType::Flash(FlashMember { data: vec![] })
                }
            }
            MemberType::Ole => {
                Self::log_ole_start(number, cast_lib, chunk);

                // Try direct OLE data
                if let Some(bytes) = Self::get_first_child_bytes(member_def) {
                    if let Some(cm) = Self::try_parse_swf(bytes, number, chunk) {
                        return cm;
                    }
                }

                // Try all XMedia children for SWF or fonts
                if let Some(cm) = Self::scan_children_for_ole(member_def, number, chunk, bitmap_manager) {
                    return cm;
                }

                // Fallback
                Self::log_unknown_ole(number, chunk);
                CastMemberType::Unknown
            }
            MemberType::Bitmap => {
                let bitmap_info = chunk.specific_data.bitmap_info().unwrap();

                let script_id = chunk
                    .member_info
                    .as_ref()
                    .map(|info| info.header.script_id)
                    .unwrap_or(0);
                
                let behavior_script_ref = if script_id > 0 {
                    let script_chunk = &lctx.as_ref().unwrap().scripts[&script_id];

                    // Create the behavior script reference
                    Some(CastMemberRef {
                        cast_lib: cast_lib as i32,
                        cast_member: script_id as i32,
                    })
                } else {
                    None
                };

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
                        debug!(
                            "Found JPEG data in Media chunk for bitmap {}, size: {} bytes",
                            number,
                            media.audio_data.len()
                        );

                        match decode_jpeg_bitmap(&media.audio_data, &bitmap_info) {
                            Ok(new_bitmap) => {
                                debug!(
                                    "Successfully decoded JPEG: {}x{}, bit_depth: {}",
                                    new_bitmap.width, new_bitmap.height, new_bitmap.bit_depth
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

                debug!(
                        "BitmapMember created → name: {} palette_id {} useAlpha {} trimWhiteSpace {}",
                        chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                        bitmap_info.palette_id,
                        bitmap_info.use_alpha,
                        bitmap_info.trim_white_space
                    );

                CastMemberType::Bitmap(BitmapMember {
                    image_ref: new_bitmap_ref,
                    reg_point: (bitmap_info.reg_x, bitmap_info.reg_y),
                    script_id,
                    member_script_ref: behavior_script_ref,
                    info: bitmap_info.clone(),
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
            MemberType::Shape => {
                if !member_def.children.is_empty() {
                    web_sys::console::log_1(&format!(
                        "(2)CastMember {} has {} children:",
                        number,
                        member_def.children.len()
                    ).into());

                    for (i, c_opt) in member_def.children.iter().enumerate() {
                        match c_opt {
                            Some(c) => web_sys::console::log_1(&format!("child[{}] = {}", i, Self::chunk_type_name(c)).into()),
                            None => web_sys::console::log_1(&format!("child[{}] = None", i).into()),
                        }
                    }
                }

                web_sys::console::log_1(&format!("Shape member {}", number).into());

                CastMemberType::Shape(ShapeMember {
                    shape_info: chunk.specific_data.shape_info().unwrap().clone(),
                })
            }
            MemberType::FilmLoop => {
                // let score_chunk = member_def.children[0].as_ref().unwrap().as_score().unwrap();
                // let film_loop_info = chunk.specific_data.film_loop_info().unwrap();
                // let mut score = Score::empty();
                // score.load_from_score_chunk(score_chunk);
                // CastMemberType::FilmLoop(FilmLoopMember {
                //     info: film_loop_info.clone(),
                //     score_chunk: score_chunk.clone(),
                //     score,
                // })
                CastMemberType::Unknown
            }
            MemberType::Sound => {
                // Log children
                if !member_def.children.is_empty() {
                    debug!(
                        "CastMember {} has {} children:",
                        number,
                        member_def.children.len()
                    );

                    for (i, c_opt) in member_def.children.iter().enumerate() {
                        match c_opt {
                            Some(c) => debug!("child[{}] = {}", i, Self::chunk_type_name(c)),
                            None => debug!("child[{}] = None", i),
                        }
                    }
                }

                // Try to find a sound chunk
                let sound_chunk_opt = member_def.children.iter()
                .filter_map(|c_opt| c_opt.as_ref())
                .find_map(|chunk| match chunk {
                    Chunk::Sound(s) => {
                    debug!("Found Sound chunk with {} bytes", s.data().len());
                    Some(s.clone())
                    },
                    Chunk::Media(m) => {
                    debug!("Found Media chunk: sample_rate={}, data_size_field={}, audio_data.len()={}, is_compressed={}",
                        m.sample_rate, m.data_size_field, m.audio_data.len(), m.is_compressed
                    );

                    // Check if the Media chunk has any sound data
                    // Don't just check is_empty - also check data_size_field
                    if !m.audio_data.is_empty() || m.data_size_field > 0 {
                        let sound = SoundChunk::from_media(&m);
                        debug!(
                        "Created SoundChunk from Media: {} bytes, rate={}",
                        sound.data().len(),
                        sound.sample_rate()
                        );
                        Some(sound)
                    } else {
                        debug!("Media chunk has no audio data");
                        None
                    }
                    },
                    _ => None,
                });

                let found_sound = sound_chunk_opt.is_some();
                debug!(
                    "CastMember {}: {} children, found sound chunk = {}",
                    number,
                    member_def.children.len(),
                    found_sound
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
                        loop_enabled: chunk
                            .member_info
                            .as_ref()
                            .map_or(false, |info| (info.header.flags & 0x10) == 0),
                    };

                    debug!(
                        "SoundMember created → name: {}, version: {}, sample_rate: {}, sample_size: {}, channels: {}, sample_count: {}, duration: {:.3}ms",
                        chunk.member_info.as_ref().map(|x| x.name.to_owned()).unwrap_or_default(),
                        sound_chunk.version,
                        info.sample_rate,
                        info.sample_size,
                        info.channels,
                        info.sample_count,
                        info.duration
                    );

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

                warn!(
                    "[CastMember::from] Unknown member type for member #{} (cast_lib={}): {:?} (id={})",
                    number,
                    cast_lib,
                    chunk.member_type, // this prints name, e.g. Button
                    member_type_id      // this prints numeric id, e.g. 15
                );

                if let Some(info) = &chunk.member_info {
                    debug!(
                        "  → name='{}', script_id={}, flags={:?}",
                        info.name, info.header.script_id, info.header.flags
                    );
                } else {
                    debug!("  → No member_info available");
                }

                // Log all child chunks
                if member_def.children.is_empty() {
                    debug!("  → No children found.");
                } else {
                    debug!("  → {} children:", member_def.children.len());

                    for (i, c_opt) in member_def.children.iter().enumerate() {
                        match c_opt {
                            Some(c) => debug!("    child[{}] = {}", i, Self::chunk_type_name(c)),
                            None => debug!("    child[{}] = None", i),
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
