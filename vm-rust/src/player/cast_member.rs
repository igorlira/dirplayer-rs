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
    chunks::{cast_member::CastMemberDef, score::{ScoreChunk, ScoreChunkHeader, ScoreFrameData}, xmedia::PfrFont, xmedia::XMediaChunk, sound::SoundChunk, Chunk, cast_member::CastMemberChunk},
    enums::{
        BitmapInfo, FilmLoopInfo, FontInfo, MemberType, ScriptType, ShapeInfo, TextMemberData, SoundInfo, FieldInfo, TextInfo,
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
pub enum Media {
    Field(FieldMember),
}

#[derive(Clone, Default)]
pub struct FieldMember {
    pub text: String,
    pub alignment: String,
    pub word_wrap: bool,
    pub font: String,
    pub font_style: String,
    pub font_size: u16,
    pub font_id: Option<u16>, // STXT font ID for lookup by ID
    pub text_height: u16,  // Text area height from FieldInfo (for dimension calculations)
    pub fixed_line_space: u16,  // Line spacing for text rendering
    pub top_spacing: i16,
    pub box_type: String,
    pub anti_alias: bool,
    pub width: u16,
    pub height: u16,  // Field member height from FieldInfo
    pub rect_left: i16,   // Initial rect from FieldInfo
    pub rect_top: i16,
    pub rect_right: i16,
    pub rect_bottom: i16,
    pub auto_tab: bool, // Tabbing order depends on sprite number order, not position on the Stage.
    pub editable: bool,
    pub border: u16,
    pub margin: u16,
    pub box_drop_shadow: u16,
    pub drop_shadow: u16,
    pub scroll_top: u16,
    pub hilite: bool,
    pub fore_color: Option<ColorRef>,  // From STXT formatting run color (>> 8)
    pub back_color: Option<ColorRef>,  // From FieldInfo bg RGB (& 0xff)
}

#[derive(Clone)]
pub struct TextMember {
    pub text: String,
    pub html_source: String,  // Original HTML string when set via html property
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
    pub height: u16,
    pub html_styled_spans: Vec<StyledSpan>,
    pub info: Option<TextInfo>,
}

pub struct PfrBitmap {
    pub bitmap_ref: BitmapRef,
    pub char_width: u16,
    pub char_height: u16,
    pub grid_columns: u8,
    pub grid_rows: u8,
    pub char_widths: Option<Vec<u16>>,
    pub first_char: u8,
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
            font_id: None,
            text_height: 100,
            fixed_line_space: 0,
            top_spacing: 0,
            box_type: "adjust".to_string(),
            anti_alias: false,
            width: 100,
            height: 100,
            rect_left: 0,
            rect_top: 0,
            rect_right: 100,
            rect_bottom: 100,
            auto_tab: false,
            editable: false,
            border: 0,
            margin: 0,
            box_drop_shadow: 0,
            drop_shadow: 0,
            scroll_top: 0,
            hilite: false,
            fore_color: None,
            back_color: None,
        }
    }

    pub fn from_field_info(field_info: &FieldInfo) -> FieldMember {
        let (bg_r, bg_g, bg_b) = field_info.bg_color_rgb();
        // bgpal all zeros = "no background color set" (transparent), not black
        let back_color = if field_info.bgpal_r == 0 && field_info.bgpal_g == 0 && field_info.bgpal_b == 0 {
            None
        } else {
            Some(ColorRef::Rgb(bg_r, bg_g, bg_b))
        };
        FieldMember {
            text: "".to_string(),
            alignment: field_info.alignment_str(),
            word_wrap: field_info.wordwrap(),
            font: field_info.font_name().to_string(),
            font_style: "plain".to_string(),
            font_size: 12,
            font_id: None,
            text_height: field_info.text_height,  // Text area height for dimension calculations
            fixed_line_space: 0,  // Use default line spacing for text rendering
            top_spacing: field_info.scroll as i16,
            box_type: field_info.box_type_str(),
            anti_alias: false,
            width: field_info.width(),  // Calculated from rect
            height: (field_info.text_height + 2 * field_info.border as u16 + 2 * field_info.margin as u16),  // Member height: text_height + borders + margins
            rect_left: field_info.rect_left,
            rect_top: field_info.rect_top,
            rect_right: field_info.rect_right,
            rect_bottom: field_info.rect_bottom,
            auto_tab: field_info.auto_tab(),
            editable: field_info.editable(),
            border: field_info.border as u16,
            margin: field_info.margin as u16,
            box_drop_shadow: field_info.box_drop_shadow as u16,
            drop_shadow: field_info.text_shadow as u16,
            scroll_top: field_info.scroll,
            hilite: false,
            fore_color: None, // Set later from STXT formatting run
            back_color,
        }
    }
}

impl TextMember {
    pub fn new() -> TextMember {
        TextMember {
            text: "".to_string(),
            html_source: String::new(),
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
            height: 20,
            html_styled_spans: Vec::new(),
            info: None,
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
    pub current_frame: u32,
    /// The bounding rectangle encompassing all sprites in the filmloop.
    /// Used to translate sprite coordinates when rendering.
    pub initial_rect: super::geometry::IntRect,
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
    pub char_widths: Option<Vec<u16>>,
    pub first_char_num: Option<u8>,
    pub alignment: TextAlignment,
    pub pfr_parsed: Option<crate::director::chunks::pfr1::types::Pfr1ParsedFont>,
    pub pfr_data: Option<Vec<u8>>,
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

#[derive(Debug, PartialEq)]
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

    /// Compute the initial bounding rectangle for a filmloop by finding the
    /// bounding box of all sprites across all frames.
    ///
    /// The coordinate system for filmloop sprites is relative to this initial_rect.
    /// When rendering, sprite positions are translated by subtracting initial_rect.left/top.
    fn compute_filmloop_initial_rect(
        frame_channel_data: &[(u32, u16, crate::director::chunks::score::ScoreFrameChannelData)],
        _reg_point: (i16, i16),
    ) -> super::geometry::IntRect {
        let mut min_x = i32::MAX;
        let mut min_y = i32::MAX;
        let mut max_x = i32::MIN;
        let mut max_y = i32::MIN;
        let mut found_any = false;

        for (_frame_idx, channel_idx, data) in frame_channel_data.iter() {
            // Skip effect channels (channels 0-5 in the raw data)
            // Real sprite channels start at index 6
            if *channel_idx < 6 {
                continue;
            }

            // Skip empty sprites (no cast member assigned)
            // Also skip sprites with cast_lib == 0 which are typically invalid/placeholder entries
            // (cast_lib 65535 is valid - it's used for internal/embedded casts)
            if data.cast_member == 0 || data.cast_lib == 0 || (data.width == 0 && data.height == 0) {
                continue;
            }

            // The sprite's position (pos_x, pos_y) is its loc (registration point location).
            // In Director, loc is where the reg point is placed.
            // Since we don't have access to cast members here, we assume CENTER registration
            // which is the default for bitmaps. This means:
            //   sprite_left = pos_x - width/2
            //   sprite_top = pos_y - height/2
            let reg_offset_x = data.width as i32 / 2;
            let reg_offset_y = data.height as i32 / 2;
            let sprite_left = data.pos_x as i32 - reg_offset_x;
            let sprite_top = data.pos_y as i32 - reg_offset_y;
            let sprite_right = sprite_left + data.width as i32;
            let sprite_bottom = sprite_top + data.height as i32;

            debug!(
                "FilmLoop initial_rect: frame {} channel {} cast {}:{} pos ({}, {}) size {}x{} -> bounds ({}, {}, {}, {})",
                _frame_idx, channel_idx, data.cast_lib, data.cast_member,
                data.pos_x, data.pos_y, data.width, data.height,
                sprite_left, sprite_top, sprite_right, sprite_bottom
            );

            if sprite_left < min_x {
                min_x = sprite_left;
            }
            if sprite_top < min_y {
                min_y = sprite_top;
            }
            if sprite_right > max_x {
                max_x = sprite_right;
            }
            if sprite_bottom > max_y {
                max_y = sprite_bottom;
            }
            found_any = true;
        }

        if !found_any {
            // No sprites found, return a default rect at origin
            debug!("FilmLoop initial_rect: no sprites found, using default (0, 0, 1, 1)");
            return super::geometry::IntRect::from(0, 0, 1, 1);
        }

        debug!(
            "FilmLoop initial_rect computed: ({}, {}, {}, {})",
            min_x, min_y, max_x, max_y
        );
        super::geometry::IntRect::from(min_x, min_y, max_x, max_y)
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

            // if 03 not found â†’ no valid text block
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

    fn resolve_font_name(chunk: &CastMemberChunk, pfr: &Option<PfrFont>, number: u32) -> String {
        if let Some(name) = chunk.member_info.as_ref().map(|i| i.name.clone()).filter(|n| !n.is_empty()) {
            return name;
        }

        if let Some(ref pfr) = pfr {
            if !pfr.font_name.is_empty() {
                return pfr.font_name.clone();
            }
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
        target_height: usize,
    ) -> PfrBitmap {
        use crate::director::chunks::pfr1::{rasterizer, parse_pfr1_font_with_target};

        // Parse at the actual target height so zone tables produce correct
        // piecewise-linear interpolation for this specific size.
        let parsed_for_size = match parse_pfr1_font_with_target(&pfr.raw_data, target_height as i32) {
            Ok(p) => p,
            Err(_) => pfr.parsed.clone(),
        };

        // Use the PFR1 rasterizer to render the parsed font
        let rasterized = rasterizer::rasterize_pfr1_font(&parsed_for_size, target_height);

        let bitmap_width = rasterized.bitmap_width as u16;
        let bitmap_height = rasterized.bitmap_height as u16;

        debug!(
            "🎨 Creating bitmap for PFR font '{}' ({}x{}, grid {}x{}, cell {}x{})",
            pfr.font_name,
            bitmap_width,
            bitmap_height,
            rasterized.grid_columns,
            rasterized.grid_rows,
            rasterized.cell_width,
            rasterized.cell_height,
        );

        // Create a 32-bit bitmap from the rasterized RGBA data
        let mut bitmap = Bitmap::new(
            bitmap_width,
            bitmap_height,
            32,
            32,
            0,
            PaletteRef::BuiltIn(BuiltInPalette::SystemWin),
        );

        // Copy RGBA data
        let data_len = rasterized.bitmap_data.len().min(bitmap.data.len());
        bitmap.data[..data_len].copy_from_slice(&rasterized.bitmap_data[..data_len]);

        // Ensure transparent background is white (avoids black-square artifacts in text rendering)
        for i in (0..data_len).step_by(4) {
            let a = bitmap.data[i + 3];
            if a == 0 {
                bitmap.data[i] = 255;
                bitmap.data[i + 1] = 255;
                bitmap.data[i + 2] = 255;
            }
        }

        bitmap.use_alpha = true;

        debug!("✅ Finished assembling PFR bitmap ({} glyphs rendered).",
            parsed_for_size.glyphs.len() + parsed_for_size.bitmap_glyphs.len());

        let bitmap_ref = bitmap_manager.add_bitmap(bitmap);

        PfrBitmap {
            bitmap_ref,
            char_width: rasterized.cell_width as u16,
            char_height: rasterized.cell_height as u16,
            grid_columns: rasterized.grid_columns as u8,
            grid_rows: rasterized.grid_rows as u8,
            char_widths: Some(rasterized.char_widths),
            first_char: rasterized.first_char,
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

            let member_name = chunk.member_info.as_ref().map(|i| i.name.as_str()).unwrap_or("");
            web_sys::console::log_1(&format!("Checking XMedia child (member #{}, name='{}', {} bytes)", number, member_name, xm.raw_data.len()).into());

            // 1) If SWF: return SWF
            if let Some(cm) = Self::try_parse_swf(xm.raw_data.to_vec(), number, chunk) {
                web_sys::console::log_1(&"Detected as SWF".into());
                return Some(cm);
            }

            // 2) Check if styled text (XMED format)
            if let Some(styled_text) = xm.parse_styled_text() {
                web_sys::console::log_1(&"Detected as XMED styled text".into());
                return Some(Self::create_text_member_from_xmed(
                    number,
                    chunk,
                    styled_text,
                ));
            }

            // 3) Font logic
            web_sys::console::log_1(&"Falling through to font parsing".into());
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
        let pfr = Self::extract_pfr(member_def);

        // Only extract preview text if this is NOT a PFR font.
        // PFR binary data contains byte patterns (0x2C...0x03) that
        // extract_text_from_xmedia misidentifies as "preview text",
        // which then causes load_fonts_into_manager to skip the font.
        let preview_text = if pfr.is_some() {
            String::new()
        } else {
            Self::extract_text_from_xmedia(&xm.raw_data)
                .filter(|s| s.len() > 3)
                .unwrap_or_default()
        };
        let preview_font_name = Self::scan_font_name_from_xmedia(xm);
        let font_name = Self::resolve_font_name(chunk, &pfr, number);

        let info_and_bitmap = Self::build_font_info_and_bitmap(
            pfr,
            chunk,
            &font_name,
            bitmap_manager,
        );

        let (font_info, bitmap_ref, char_w, char_h, gc, gr, char_widths, first_char, pfr_parsed, pfr_data) = info_and_bitmap;

        let member_name = chunk
            .member_info
            .as_ref()
            .map(|x| x.name.to_owned())
            .unwrap_or_default();

        debug!(
            "FontMember #{} name='{}' font_name='{}' preview_text='{}' preview_font_name={:?} \
             fixed_line_space=14 top_spacing=0 char_width={:?} char_height={:?} \
             grid_columns={:?} grid_rows={:?} first_char_num={:?} char_widths_len={} \
             bitmap_ref={:?} pfr_parsed={} pfr_data_len={}",
            number, member_name, font_name, preview_text, preview_font_name,
            char_w, char_h, gc, gr, first_char,
            char_widths.as_ref().map_or(0, |v| v.len()),
            bitmap_ref, pfr_parsed.is_some(), pfr_data.as_ref().map_or(0, |d| d.len()),
        );

        CastMember {
            number,
            name: member_name,
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
                char_widths,
                first_char_num: first_char,
                alignment: TextAlignment::Left,
                pfr_parsed,
                pfr_data,
            }),
            color: ColorRef::PaletteIndex(255),
            bg_color: ColorRef::PaletteIndex(0),
        }
    }

    fn create_text_member_from_xmed(
        number: u32,
        chunk: &CastMemberChunk,
        styled_text: crate::director::chunks::xmedia::XmedStyledText,
    ) -> CastMember {
        use crate::player::handlers::datum_handlers::cast_member::font::TextAlignment;

        debug!("Creating TextMember from XMED styled text (member #{})", number);

        let alignment_str = match styled_text.alignment {
            TextAlignment::Left => "left",
            TextAlignment::Center => "center",
            TextAlignment::Right => "right",
            TextAlignment::Justify => "justify",
        };

        // Use first span font face, but member fontSize should track the largest styled size.
        let (font_name, font_size) = if !styled_text.styled_spans.is_empty() {
            let first_style = &styled_text.styled_spans[0].style;
            let max_span_size = styled_text
                .styled_spans
                .iter()
                .filter_map(|s| s.style.font_size)
                .filter(|s| *s > 0)
                .max()
                .unwrap_or(12);
            (
                first_style.font_face.clone().unwrap_or_else(|| "Arial".to_string()),
                max_span_size as u16,
            )
        } else {
            ("Arial".to_string(), 12)
        };

        debug!(
            "  Text: '{}', alignment: {}, font: {}, size: {}, spans: {}, word_wrap: {}",
            styled_text.text, alignment_str, font_name, font_size, styled_text.styled_spans.len(),
            styled_text.word_wrap
        );

        // Get TextInfo from specific_data if available; otherwise synthesize a default one
        // so runtime properties like centerRegPoint are always present on parsed text members.
        let text_info_from_chunk = chunk.specific_data.text_info().cloned();
        let raw_looks_like_text_info = TextInfo::looks_like_text_info(chunk.specific_data_raw.as_slice());
        let text_info_from_raw = if text_info_from_chunk.is_none() && raw_looks_like_text_info {
            Some(TextInfo::from(chunk.specific_data_raw.as_slice()))
        } else {
            None
        };
        let text_info_from_chunk = text_info_from_chunk.or(text_info_from_raw);
        let field_info_from_chunk = chunk.specific_data.field_info();
        let mut text_info = text_info_from_chunk.unwrap_or_else(|| {
            let mut info = TextInfo::default();
            if let Some(field_info) = field_info_from_chunk {
                info.box_type = field_info.box_type as u32;
                info.scroll_top = field_info.scroll as u32;
                info.auto_tab = field_info.auto_tab();
                info.editable = field_info.editable();
                info.width = field_info.width() as u32;
                info.height = field_info.height() as u32;
            }
            info
        });
        let mut box_w = if text_info.width > 0 { text_info.width as u16 } else { 0 };
        let mut box_h = if text_info.height > 0 { text_info.height as u16 } else { 0 };

        // Fallback for older text member formats: parse raw text member data for dimensions.
        if box_w == 0 || box_h == 0 {
            if let Some(text_member_data) = TextMemberData::from_raw_bytes(chunk.specific_data_raw.as_slice()) {
                if box_w == 0 && text_member_data.width > 0 {
                    box_w = text_member_data.width as u16;
                }
                if box_h == 0 && text_member_data.height > 0 {
                    box_h = text_member_data.height as u16;
                }
            }
        }

        if box_w == 0 { box_w = 100; }
        if box_h == 0 { box_h = 20; }

        // Keep synthesized TextInfo dimensions aligned with effective member box.
        text_info.width = box_w as u32;
        text_info.height = box_h as u32;

        let box_type = text_info.box_type_str().trim_start_matches('#').to_string();
        let text_member = TextMember {
            text: styled_text.text.clone(),
            html_source: String::new(),
            alignment: alignment_str.to_string(),
            box_type,
            word_wrap: styled_text.word_wrap,
            anti_alias: true,
            font: font_name,
            font_style: Vec::new(),
            font_size,
            fixed_line_space: if styled_text.line_spacing > 0 {
                styled_text.line_spacing as u16
            } else {
                styled_text.fixed_line_space
            },
            top_spacing: styled_text.top_spacing as i16,
            width: box_w,
            height: box_h,
            html_styled_spans: styled_text.styled_spans,
            info: Some(text_info),
        };

        let member_name = chunk
            .member_info
            .as_ref()
            .map(|x| x.name.to_owned())
            .unwrap_or_default();

        debug!(
            "TextMember #{} name='{}' text='{}' alignment='{}' box_type='{}' word_wrap={} \
             anti_alias={} font='{}' font_style={:?} font_size={} fixed_line_space={} \
             top_spacing={} width={} height={} styled_spans={}",
            number,
            member_name,
            text_member.text,
            text_member.alignment,
            text_member.box_type,
            text_member.word_wrap,
            text_member.anti_alias,
            text_member.font,
            text_member.font_style,
            text_member.font_size,
            text_member.fixed_line_space,
            text_member.top_spacing,
            text_member.width,
            text_member.height,
            text_member.html_styled_spans.len(),
        );

        // Preserve XMED foreColor at the member level so it persists
        // even when Lingo sets member.text or member.html (which may clear styled span colors)
        let member_color = text_member.html_styled_spans.first()
            .and_then(|s| s.style.color)
            .map(|c| ColorRef::Rgb(
                ((c >> 16) & 0xFF) as u8,
                ((c >> 8) & 0xFF) as u8,
                (c & 0xFF) as u8,
            ))
            .unwrap_or(ColorRef::PaletteIndex(255));

        CastMember {
            number,
            name: member_name,
            member_type: CastMemberType::Text(text_member),
            color: member_color,
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
        Option<Vec<u16>>, // char widths
        Option<u8>, // first_char_num
        Option<crate::director::chunks::pfr1::types::Pfr1ParsedFont>,
        Option<Vec<u8>>,
        ) {
        let specific_bytes = chunk.specific_data_raw.clone();

        if let Some(pfr) = pfr {
            let requested_size = chunk
                .specific_data
                .font_info()
                .map(|fi| fi.size)
                .unwrap_or(0);
            let target_height = if requested_size > 0 {
                requested_size as usize
            } else {
                16usize
            };

            let bmp = Self::render_pfr_to_bitmap(&pfr, bitmap_manager, target_height);

            let info = FontInfo {
                font_id: 0,
                size: target_height as u16,
                style: 0,
                name: if pfr.font_name.is_empty() {
                    default_name.to_string()
                } else {
                    pfr.font_name.clone()
                }
            };

            web_sys::console::log_1(&format!("Rendered PFR: {:?}", info).into());

            return (
                info,
                Some(bmp.bitmap_ref),
                Some(bmp.char_width),
                Some(bmp.char_height),
                Some(bmp.grid_columns),
                Some(bmp.grid_rows),
                bmp.char_widths,
                Some(bmp.first_char),
                Some(pfr.parsed.clone()),
                Some(pfr.raw_data.clone()),
            );
        }

        if FontInfo::looks_like_real_font_data(&specific_bytes) {
            let info = chunk
                .specific_data
                .font_info()
                .map(|fi| fi.clone().with_default_name(default_name))
                .unwrap_or_else(|| FontInfo::minimal(default_name));

            return (info, None, None, None, None, None, None, None, None, None);
        }

        // fallback
        (FontInfo::minimal(default_name), None, None, None, None, None, None, None, None, None)
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
        dir_version: u16,
    ) -> CastMember {
        let chunk = &member_def.chunk;

        let member_type = match chunk.member_type {
            MemberType::Text => {
                let text_chunk = member_def.children[0]
                    .as_ref()
                    .unwrap()
                    .as_text()
                    .expect("Not a text chunk");
                let raw = chunk.specific_data_raw.as_slice();
                let field_info = FieldInfo::from(raw);
                let mut field_member = FieldMember::from_field_info(&field_info);
                field_member.text = text_chunk.text.clone();

                // Parse STXT formatting data to extract actual fontId, fontSize, and style
                let formatting_runs = text_chunk.parse_formatting_runs();
                for (i, run) in formatting_runs.iter().enumerate() {
                    debug!(
                        "  formatting_run[{}]: start_position={} height={} ascent={} font_id={} style=0x{:02X} font_size={} color=({},{},{}) -> rgb({},{},{})",
                        i, run.start_position, run.height, run.ascent, run.font_id, run.style, run.font_size,
                        run.color_r, run.color_g, run.color_b,
                        (run.color_r >> 8) as u8, (run.color_g >> 8) as u8, (run.color_b >> 8) as u8,
                    );
                }
                if let Some(first_run) = formatting_runs.first() {
                    // Extract foreground color from STXT formatting run
                    let fg_r = (first_run.color_r >> 8) as u8;
                    let fg_g = (first_run.color_g >> 8) as u8;
                    let fg_b = (first_run.color_b >> 8) as u8;
                    field_member.fore_color = Some(ColorRef::Rgb(fg_r, fg_g, fg_b));

                    field_member.font_id = Some(first_run.font_id);
                    if first_run.font_size > 0 {
                        field_member.font_size = first_run.font_size;
                        // Ensure field line height can fit the parsed font size.
                        if field_member.fixed_line_space < first_run.font_size {
                            field_member.fixed_line_space = first_run.font_size;
                        }
                    }
                    if first_run.style != 0 {
                        let mut styles = Vec::new();
                        if (first_run.style & 0x01) != 0 {
                            styles.push("bold");
                        }
                        if (first_run.style & 0x02) != 0 {
                            styles.push("italic");
                        }
                        if (first_run.style & 0x04) != 0 {
                            styles.push("underline");
                        }
                        if styles.is_empty() {
                            field_member.font_style = "plain".to_string();
                        } else {
                            field_member.font_style = styles.join(" ");
                        }
                    }
                }

                debug!(
                    "FieldMember text='{}' alignment='{}' word_wrap={} font='{}' \
                     font_style='{}' font_size={} font_id={:?} fixed_line_space={} \
                     top_spacing={} box_type='{}' anti_alias={} width={} \
                     auto_tab={} editable={} border={} fore_color={:?} back_color={:?} formatting_runs={}",
                    field_member.text, field_member.alignment, field_member.word_wrap,
                    field_member.font, field_member.font_style, field_member.font_size,
                    field_member.font_id, field_member.fixed_line_space,
                    field_member.top_spacing, field_member.box_type, field_member.anti_alias,
                    field_member.width, field_member.auto_tab, field_member.editable,
                    field_member.border, field_member.fore_color, field_member.back_color,
                    formatting_runs.len(),
                );

                CastMemberType::Field(field_member)
            }
            MemberType::Script => {
                let member_info = chunk.member_info.as_ref().unwrap();
                let script_id = member_info.header.script_id;
                let script_type = chunk.specific_data.script_type().unwrap();
                let has_script = lctx.as_ref()
                    .map(|ctx| ctx.scripts.contains_key(&script_id))
                    .unwrap_or(false);

                if has_script {
                    CastMemberType::Script(ScriptMember {
                        script_id,
                        script_type,
                        name: member_info.name.clone(),
                    })
                } else {
                    web_sys::console::warn_1(&format!("Script member {}: script_id {} not found in Lctx, skipping", number, script_id).into());
                    CastMemberType::Unknown
                }
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
                        "BitmapMember created â†’ name: {} palette_id {} useAlpha {} trimWhiteSpace {}",
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
                let score_chunk_opt = member_def.children.get(0)
                    .and_then(|c| c.as_ref())
                    .and_then(|c| c.as_score());
                let film_loop_info = chunk.specific_data.film_loop_info().unwrap();

                if let Some(score_chunk) = score_chunk_opt {
                    let mut score = Score::empty();
                    score.load_from_score_chunk(score_chunk, dir_version);

                    // Compute initial_rect by finding the bounding box of all sprites
                    let initial_rect = Self::compute_filmloop_initial_rect(
                        &score_chunk.frame_data.frame_channel_data,
                        film_loop_info.reg_point,
                    );

                    debug!(
                        "FilmLoop {} initial_rect: ({}, {}, {}, {}), info size: {}x{}, reg_point: ({}, {})",
                        number,
                        initial_rect.left, initial_rect.top, initial_rect.right, initial_rect.bottom,
                        film_loop_info.width, film_loop_info.height,
                        film_loop_info.reg_point.0, film_loop_info.reg_point.1
                    );

                    // Log sprite_spans info
                    debug!(
                        "FilmLoop {} has {} sprite_spans, {} frame_intervals, {} frame_channel_data entries",
                        number,
                        score.sprite_spans.len(),
                        score_chunk.frame_intervals.len(),
                        score_chunk.frame_data.frame_channel_data.len()
                    );

                    CastMemberType::FilmLoop(FilmLoopMember {
                        info: film_loop_info.clone(),
                        score_chunk: score_chunk.clone(),
                        score,
                        current_frame: 1, // Start at frame 1
                        initial_rect,
                    })
                } else {
                    warn!("FilmLoop {} has no valid score chunk, creating empty film loop", number);
                    let empty_score_chunk = ScoreChunk {
                        header: ScoreChunkHeader {
                            total_length: 0, unk1: 0, unk2: 0,
                            entry_count: 0, unk3: 0, entry_size_sum: 0,
                        },
                        entries: vec![],
                        frame_intervals: vec![],
                        frame_data: ScoreFrameData::default(),
                        sprite_details: std::collections::HashMap::new(),
                    };
                    CastMemberType::FilmLoop(FilmLoopMember {
                        info: film_loop_info.clone(),
                        score_chunk: empty_score_chunk,
                        score: Score::empty(),
                        current_frame: 1,
                        initial_rect: super::geometry::IntRect { left: 0, top: 0, right: 0, bottom: 0 },
                    })
                }
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
                        "SoundMember created â†’ name: {}, version: {}, sample_rate: {}, sample_size: {}, channels: {}, sample_count: {}, duration: {:.3}ms",
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
                        "  â†’ name='{}', script_id={}, flags={:?}",
                        info.name, info.header.script_id, info.header.flags
                    );
                } else {
                    debug!("  â†’ No member_info available");
                }

                // Log all child chunks
                if member_def.children.is_empty() {
                    debug!("  â†’ No children found.");
                } else {
                    debug!("  â†’ {} children:", member_def.children.len());

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
