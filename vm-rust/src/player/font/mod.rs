use rustc_hash::FxHashMap;
use log::{warn, debug};
use std::cell::Cell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::player::{
    bitmap::bitmap::{get_system_default_palette, Bitmap, PaletteRef},
    cast_member::CastMemberType,
    reserve_player_mut, CastManager,
};

use std::collections::HashMap;

use crate::director::enums::FontInfo;

use super::{
    bitmap::{drawing::CopyPixelsParams, manager::BitmapRef, palette_map::PaletteMap},
    geometry::IntRect,
};

/// Controls how text glyphs are rendered for PFR fonts.
/// Can be toggled at runtime via `set_glyph_preference()` from JS.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum GlyphPreference {
    /// Current default behavior: PFR fonts use bitmap atlas, standard fonts use Canvas2D native.
    Auto,
    /// Force bitmap atlas rendering for all fonts (PFR rasterized glyphs or system font bitmap).
    Bitmap,
    /// Force Canvas2D native text rendering (fillText) even for PFR fonts.
    Native,
    /// Force outline-only rasterization for PFR fonts (skip bitmap strikes).
    /// Requires font cache clear + re-rasterization to take effect.
    Outline,
}

thread_local! {
    static GLYPH_PREFERENCE: Cell<GlyphPreference> = Cell::new(GlyphPreference::Auto);
}

pub fn get_glyph_preference() -> GlyphPreference {
    GLYPH_PREFERENCE.with(|pref| pref.get())
}

pub fn set_glyph_preference(pref: GlyphPreference) {
    GLYPH_PREFERENCE.with(|p| p.set(pref));
    debug!("[GlyphPreference] Set to {:?}", pref);
}

pub type FontRef = u32;

pub struct FontManager {
    pub fonts: FxHashMap<FontRef, Rc<BitmapFont>>,
    pub system_font: Option<Rc<BitmapFont>>,
    pub font_counter: FontRef,
    pub font_cache: HashMap<String, Rc<BitmapFont>>, // Cache for loaded fonts by name
    pub font_by_id: HashMap<u16, FontRef>,           // Map font_id to FontRef
    pub default_pfr_data: HashMap<String, &'static [u8]>, // Embedded default PFR fonts
}

#[derive(Clone, Debug)]
pub struct BitmapFont {
    pub bitmap_ref: BitmapRef,
    pub char_width: u16,
    pub char_height: u16,
    pub grid_columns: u8,
    pub grid_rows: u8,
    pub grid_cell_width: u16,
    pub grid_cell_height: u16,
    pub char_offset_x: u16,
    pub char_offset_y: u16,
    pub first_char_num: u8,
    pub font_name: String,
    pub font_size: u16,
    pub font_style: u8,
    pub char_widths: Option<Vec<u16>>,
    pub pfr_native_size: u16,
}

impl BitmapFont {
    /// Get the advance width for a specific character.
    /// Falls back to uniform char_width if no proportional widths are available.
    pub fn get_char_advance(&self, char_num: u8) -> u16 {
        if let Some(ref widths) = self.char_widths {
            let idx = char_num.saturating_sub(self.first_char_num) as usize;
            if idx < widths.len() {
                return widths[idx];
            }
        }
        self.char_width
    }
}

pub struct DrawTextParams<'a> {
    pub font: &'a BitmapFont,
    pub line_height: Option<u16>,
    pub line_spacing: u16,
    pub top_spacing: i16,
}

impl FontManager {
    pub fn canonical_font_name(name: &str) -> String {
        let normalized = name
            .trim()
            .to_ascii_lowercase()
            .replace('_', " ")
            .replace('*', " ");
        let mut parts: Vec<&str> = normalized.split_whitespace().collect();

        while let Some(last) = parts.last() {
            if last.chars().all(|c| c.is_ascii_digit()) {
                parts.pop();
            } else {
                break;
            }
        }

        parts.join(" ")
    }

    /// Get a font by name and size, rasterizing a PFR font at the requested size if needed.
    pub fn get_font_with_cast_and_bitmap(
        &mut self,
        font_name: &str,
        cast_manager: &CastManager,
        bitmap_manager: &mut crate::player::bitmap::manager::BitmapManager,
        size: Option<u16>,
        style: Option<u8>,
    ) -> Option<Rc<BitmapFont>> {
        let requested_size = size.unwrap_or(0);
        let requested_style = style.unwrap_or(0);

        // Fast path: exact size/style cache hit to avoid reparsing/rerasterizing PFR fonts.
        if requested_size > 0 {
            let lc = font_name.to_ascii_lowercase();
            let mut exact_keys = vec![
                format!("{}_{}_{}", lc, requested_size, requested_style),
                format!("{}_{}_0", lc, requested_size),
            ];
            let canon = Self::canonical_font_name(font_name);
            if !canon.is_empty() {
                exact_keys.push(format!("{}_{}_{}", canon, requested_size, requested_style));
                exact_keys.push(format!("{}_{}_0", canon, requested_size));
            }

            for key in exact_keys {
                if let Some(font) = self.font_cache.get(&key) {
                    if font.font_size == requested_size {
                        return Some(Rc::clone(font));
                    }
                }
            }
        }

        if let Some(font) = self.get_font_with_cast(font_name, Some(cast_manager), size, style) {
            if size.map_or(true, |s| s == font.font_size) {
                return Some(font);
            }
        }

        if requested_size == 0 {
            return self.get_font_with_cast(font_name, Some(cast_manager), size, style);
        }

        let font_name_lc = font_name.to_lowercase();
        let font_name_canon = Self::canonical_font_name(font_name);
        for cast_lib in &cast_manager.casts {
            for member in cast_lib.members.values() {
                if let CastMemberType::Font(font_data) = &member.member_type {
                    let info_name_canon = Self::canonical_font_name(&font_data.font_info.name);
                    let member_name_canon = Self::canonical_font_name(&member.name);
                    let name_matches =
                        font_data.font_info.name.to_lowercase() == font_name_lc
                            || member.name.to_lowercase() == font_name_lc
                            || (!font_name_canon.is_empty()
                                && (info_name_canon == font_name_canon
                                    || member_name_canon == font_name_canon));
                    if !name_matches {
                        continue;
                    }

                    if let Some(ref parsed) = font_data.pfr_parsed {
                        use crate::director::chunks::pfr1::{rasterizer, parse_pfr1_font_with_target};

                        let parsed_for_size = if let Some(ref raw) = font_data.pfr_data {
                            match parse_pfr1_font_with_target(raw, 0) {
                                Ok(p) => p,
                                Err(_) => parsed.clone(),
                            }
                        } else {
                            parsed.clone()
                        };

                        let rasterized = rasterizer::rasterize_pfr1_font(&parsed_for_size, requested_size as usize, font_data.font_info.size as usize);

                        let bitmap_width = rasterized.bitmap_width as u16;
                        let bitmap_height = rasterized.bitmap_height as u16;

                        let mut bitmap = Bitmap::new(
                            bitmap_width,
                            bitmap_height,
                            32,
                            32,
                            0,
                            PaletteRef::BuiltIn(get_system_default_palette()),
                        );

                        let data_len = rasterized.bitmap_data.len().min(bitmap.data.len());
                        bitmap.data[..data_len].copy_from_slice(&rasterized.bitmap_data[..data_len]);

                        for i in (0..data_len).step_by(4) {
                            let a = bitmap.data[i + 3];
                            if a == 0 {
                                bitmap.data[i] = 255;
                                bitmap.data[i + 1] = 255;
                                bitmap.data[i + 2] = 255;
                            }
                        }
                        bitmap.use_alpha = true;

                        let bitmap_ref = bitmap_manager.add_bitmap(bitmap);

                        let final_char_widths = rasterized.char_widths;

                        let font = BitmapFont {
                            bitmap_ref,
                            char_width: rasterized.cell_width as u16,
                            char_height: rasterized.cell_height as u16,
                            grid_columns: rasterized.grid_columns as u8,
                            grid_rows: rasterized.grid_rows as u8,
                            grid_cell_width: rasterized.cell_width as u16,
                            grid_cell_height: rasterized.cell_height as u16,
                            char_offset_x: 0,
                            char_offset_y: 0,
                            first_char_num: rasterized.first_char,
                            font_name: member.name.clone(),
                            font_size: requested_size,
                            font_style: font_data.font_info.style,
                            char_widths: Some(final_char_widths),
                            pfr_native_size: font_data.font_info.size,
                        };

                        let rc_font = Rc::new(font);
                        let cache_key = Self::cache_key(&format!("{}_{}_{}", font_name, requested_size, style.unwrap_or(0)));
                        self.font_cache.insert(cache_key, Rc::clone(&rc_font));
                        self.font_cache.insert(Self::cache_key(font_name), Rc::clone(&rc_font));
                        self.font_cache.insert(Self::cache_key(&member.name), Rc::clone(&rc_font));

                        let font_ref = self.font_counter;
                        self.font_counter += 1;
                        self.fonts.insert(font_ref, Rc::clone(&rc_font));
                        self.font_by_id.insert(font_data.font_info.font_id, font_ref);

                        return Some(rc_font);
                    }
                }
            }
        }

        warn!(
            "[font] No PFR re-rasterization match for '{}' at size {}",
            font_name, requested_size,
        );

        // Fallback: try default embedded PFR fonts
        if requested_size > 0 {
            let font_name_canon = Self::canonical_font_name(font_name);
            let matched_key = self.default_pfr_data.keys()
                .find(|k| {
                    k.eq_ignore_ascii_case(font_name)
                        || Self::canonical_font_name(k) == font_name_canon
                })
                .cloned();
            if let Some(key) = matched_key {
                if let Some(pfr_bytes) = self.default_pfr_data.get(&key) {
                    use crate::director::chunks::pfr1::{rasterizer, parse_pfr1_font_with_target};

                    match parse_pfr1_font_with_target(pfr_bytes, 0) {
                        Ok(parsed) => {
                            let rasterized = rasterizer::rasterize_pfr1_font(&parsed, requested_size as usize, 0);

                            let bitmap_width = rasterized.bitmap_width as u16;
                            let bitmap_height = rasterized.bitmap_height as u16;

                            let mut bitmap = Bitmap::new(
                                bitmap_width,
                                bitmap_height,
                                32,
                                32,
                                0,
                                PaletteRef::BuiltIn(get_system_default_palette()),
                            );

                            let data_len = rasterized.bitmap_data.len().min(bitmap.data.len());
                            bitmap.data[..data_len].copy_from_slice(&rasterized.bitmap_data[..data_len]);

                            for i in (0..data_len).step_by(4) {
                                let a = bitmap.data[i + 3];
                                if a == 0 {
                                    bitmap.data[i] = 255;
                                    bitmap.data[i + 1] = 255;
                                    bitmap.data[i + 2] = 255;
                                }
                            }
                            bitmap.use_alpha = true;

                            let bitmap_ref = bitmap_manager.add_bitmap(bitmap);

                            let final_char_widths = rasterized.char_widths;

                            let font = BitmapFont {
                                bitmap_ref,
                                char_width: rasterized.cell_width as u16,
                                char_height: rasterized.cell_height as u16,
                                grid_columns: rasterized.grid_columns as u8,
                                grid_rows: rasterized.grid_rows as u8,
                                grid_cell_width: rasterized.cell_width as u16,
                                grid_cell_height: rasterized.cell_height as u16,
                                char_offset_x: 0,
                                char_offset_y: 0,
                                first_char_num: rasterized.first_char,
                                font_name: key.clone(),
                                font_size: requested_size,
                                font_style: style.unwrap_or(0),
                                char_widths: Some(final_char_widths),
                                pfr_native_size: 0,
                            };

                            let rc_font = Rc::new(font);
                            let cache_key = Self::cache_key(&format!("{}_{}_{}", font_name, requested_size, style.unwrap_or(0)));
                            self.font_cache.insert(cache_key, Rc::clone(&rc_font));
                            self.font_cache.insert(Self::cache_key(font_name), Rc::clone(&rc_font));
                            self.font_cache.insert(Self::cache_key(&key), Rc::clone(&rc_font));

                            let font_ref = self.font_counter;
                            self.font_counter += 1;
                            self.fonts.insert(font_ref, Rc::clone(&rc_font));

                            debug!(
                                "Loaded default PFR font '{}' at size {}",
                                key, requested_size
                            );

                            return Some(rc_font);
                        }
                        Err(e) => {
                            debug!(
                                "Failed to parse default PFR font '{}': {}",
                                key, e
                            );
                        }
                    }
                }
            }
        }

        None
    }
    pub fn new() -> FontManager {
        return FontManager {
            fonts: FxHashMap::default(),
            system_font: None,
            font_counter: 0,
            font_cache: HashMap::new(),
            font_by_id: HashMap::new(),
            default_pfr_data: HashMap::new(),
        };
    }

    /// Normalize a font cache key to lowercase for case-insensitive lookups.
    #[inline]
    pub fn cache_key(s: &str) -> String {
        s.to_ascii_lowercase()
    }

    pub fn get_system_font(&self) -> Option<Rc<BitmapFont>> {
        self.system_font.clone()
    }

    pub fn get_font_by_info(&self, font_info: &FontInfo) -> Option<&BitmapFont> {
        // First try to get by font_id
        if let Some(font_ref) = self.font_by_id.get(&font_info.font_id) {
            return self.fonts.get(font_ref).map(|v| &**v);
        }

        // Fall back to name lookup
        self.get_font_immutable(&font_info.name)
    }

    /// Get a font by name, loading it if necessary
    pub fn get_font(&mut self, font_name: &str) -> Option<&BitmapFont> {
        // Check cache first (case-insensitive)
        let key = Self::cache_key(font_name);
        if self.font_cache.contains_key(&key) {
            return self.font_cache.get(&key).map(|v| &**v);
        }

        // Font not in cache, cannot load here (would need cast_manager)
        None
    }

    /// Load a font from cast members
    fn load_font(&mut self, font_name: &str) -> Option<BitmapFont> {
        // TODO: Implement actual font loading from cast members
        // This is a placeholder that returns None
        // You'll need to:
        // 1. Search through cast members for a Font type with matching name
        // 2. Extract font metrics from FontInfo
        // 3. Create or reference a BitmapFont

        web_sys::console::log_1(
            &format!(
                "FontManager: Attempted to load font '{}' - not implemented yet",
                font_name
            )
            .into(),
        );

        None
    }

    /// Get a font by name (immutable version)
    pub fn get_font_immutable(&self, font_name: &str) -> Option<&BitmapFont> {
        let key = Self::cache_key(font_name);
        self.font_cache.get(&key).map(|v| &**v)
    }

    /// Load a font from cast members by searching the cast manager
    pub fn load_font_from_cast(
        &mut self,
        font_name: &str,
        cast_manager: &CastManager,
        size: Option<u16>,
        style: Option<u8>,
    ) -> Option<Rc<BitmapFont>> {
        let cache_key = Self::cache_key(&format!("{}_{}_{}", font_name, size.unwrap_or(0), style.unwrap_or(0)));

        for cast_lib in &cast_manager.casts {
            for member in cast_lib.members.values() {
                if let CastMemberType::Font(font_data) = &member.member_type {
                    // Check BOTH the font_info.name AND the member.name
                    let font_name_lc = font_name.to_lowercase();
                    let font_name_canon = Self::canonical_font_name(font_name);
                    let info_name_canon = Self::canonical_font_name(&font_data.font_info.name);
                    let member_name_canon = Self::canonical_font_name(&member.name);

                    let name_matches =
                        font_data.font_info.name.to_lowercase() == font_name_lc
                        || member.name.to_lowercase() == font_name_lc
                        || (!font_name_canon.is_empty()
                            && (info_name_canon == font_name_canon
                                || member_name_canon == font_name_canon));
                    let size_matches = size.is_none() || size == Some(font_data.font_info.size);
                    let style_matches = style.is_none() || style == Some(font_data.font_info.style);

                    if name_matches && size_matches && style_matches {
                        web_sys::console::log_1(
                            &format!(
                                "Found matching font: member.name='{}', font_info.name='{}'",
                                member.name, font_data.font_info.name
                            )
                            .into(),
                        );

                        // Check if this font has a bitmap_ref from PFR parsing
                        if let Some(bitmap_ref) = font_data.bitmap_ref {
                            web_sys::console::log_1(
                                &format!("Found PFR font with bitmap_ref: {}", bitmap_ref)
                                    .into(),
                            );

                            let font = BitmapFont {
                                bitmap_ref,
                                char_width: font_data.char_width.unwrap_or(8),
                                char_height: font_data.char_height.unwrap_or(12),
                                grid_columns: font_data.grid_columns.unwrap_or(16),
                                grid_rows: font_data.grid_rows.unwrap_or(8),
                                grid_cell_width: font_data.char_width.unwrap_or(8),
                                grid_cell_height: font_data.char_height.unwrap_or(12),
                                first_char_num: font_data.first_char_num.unwrap_or(32),
                                char_offset_x: 0,
                                char_offset_y: 0,
                                font_name: member.name.clone(),
                                font_size: font_data.font_info.size,
                                font_style: font_data.font_info.style,
                                char_widths: font_data.char_widths.clone(),
                                pfr_native_size: font_data.font_info.size,
                            };

                            let rc_font = Rc::new(font);

                            // Cache under ALL name variations
                            web_sys::console::log_1(
                                &format!(
                                    "Caching font as: '{}', '{}', '{}'",
                                    cache_key, font_name, member.name
                                )
                                .into(),
                            );

                            self.font_cache
                                .insert(cache_key.clone(), Rc::clone(&rc_font));
                            self.font_cache
                                .insert(Self::cache_key(font_name), Rc::clone(&rc_font));
                            self.font_cache
                                .insert(Self::cache_key(&member.name), Rc::clone(&rc_font));

                            if font_data.font_info.name != member.name
                                && font_data.font_info.name != font_name
                            {
                                self.font_cache
                                    .insert(Self::cache_key(&font_data.font_info.name), Rc::clone(&rc_font));
                            }

                            let font_ref = self.font_counter;
                            self.font_counter += 1;
                            self.fonts.insert(font_ref, Rc::clone(&rc_font));
                            self.font_by_id
                                .insert(font_data.font_info.font_id, font_ref);

                            return Some(rc_font);
                        }

                        // Fallback to system font with scaling (shouldn't happen for PFR fonts)
                        if let Some(system_font) = self.get_system_font() {
                            let mut new_font = (*system_font).clone();
                            new_font.font_name = font_data.font_info.name.clone();
                            new_font.font_size = font_data.font_info.size;
                            new_font.font_style = font_data.font_info.style;
                            new_font.pfr_native_size = font_data.font_info.size;

                            let scale_factor = font_data.font_info.size as f32 / 12.0;
                            new_font.char_width =
                                (new_font.char_width as f32 * scale_factor) as u16;
                            new_font.char_height =
                                (new_font.char_height as f32 * scale_factor) as u16;

                            let rc_font = Rc::new(new_font);
                            self.font_cache
                                .insert(cache_key.clone(), Rc::clone(&rc_font));
                            let name_key = Self::cache_key(font_name);
                            if !self.font_cache.contains_key(&name_key) {
                                self.font_cache
                                    .insert(name_key, Rc::clone(&rc_font));
                            }

                            let font_ref = self.font_counter;
                            self.font_counter += 1;
                            self.fonts.insert(font_ref, Rc::clone(&rc_font));
                            self.font_by_id
                                .insert(font_data.font_info.font_id, font_ref);

                            return Some(rc_font);
                        }
                    }
                }
            }
        }

        None
    }

    pub fn get_font_with_cast(
        &mut self,
        font_name: &str,
        cast_manager: Option<&CastManager>,
        size: Option<u16>,
        style: Option<u8>,
    ) -> Option<Rc<BitmapFont>> {
        fn push_candidate(candidates: &mut Vec<String>, s: String) {
            if s.is_empty() {
                return;
            }
            if candidates.iter().any(|c| c == &s) {
                return;
            }
            candidates.push(s);
        }

        let mut candidates: Vec<String> = Vec::new();

        let name = font_name.to_string();
        let name_lc = font_name.to_ascii_lowercase();
        let name_canon = Self::canonical_font_name(font_name);
        push_candidate(&mut candidates, name.clone());
        push_candidate(&mut candidates, name_lc.clone());
        if !name_canon.is_empty() {
            push_candidate(&mut candidates, name_canon.clone());
        }

        // Common Director/PFR aliases
        if name.contains('_') {
            push_candidate(&mut candidates, name.replace('_', " "));
        }
        if name.contains(' ') {
            push_candidate(&mut candidates, name.replace(' ', "_"));
        }
        if name.contains('*') {
            push_candidate(&mut candidates, name.replace('*', "_"));
            push_candidate(&mut candidates, name.replace('*', " "));
            push_candidate(&mut candidates, name.replace('*', "").trim().to_string());
        }

        // Prefix before underscore/space/asterisk
        if let Some(idx) = name.find(|c: char| c == '_' || c == ' ' || c == '*') {
            let prefix = &name[..idx];
            if prefix.len() > 1 {
                push_candidate(&mut candidates, prefix.to_string());
                push_candidate(&mut candidates, prefix.to_ascii_lowercase());
            }
        }

        // Also try size/style suffixed keys used in cache
        let size_val = size.unwrap_or(0);
        let style_val = style.unwrap_or(0);
        let mut expanded: Vec<String> = Vec::new();
        for c in &candidates {
            expanded.push(format!("{}_{}_{}", c, size_val, style_val));
            expanded.push(format!("{}_{}_0", c, size_val));
        }
        for e in expanded {
            push_candidate(&mut candidates, e);
        }

        // Cache lookup (case-insensitive) for all candidates
        for c in &candidates {
            if let Some(font) = self.font_cache.get(&Self::cache_key(c)) {
                return Some(Rc::clone(font));
            }
        }

        // Canonical fallback: match normalized names across cache keys.
        if !name_canon.is_empty() {
            for (key, font) in &self.font_cache {
                if Self::canonical_font_name(key) == name_canon {
                    return Some(Rc::clone(font));
                }
            }
        }

        // Try cast loading with candidates
        if let Some(cast_mgr) = cast_manager {
            for c in &candidates {
                if let Some(font) = self.load_font_from_cast(c, cast_mgr, size, style) {
                    return Some(font);
                }
            }
        }

        None
    }

    pub fn get_best_font(&mut self, font_name: &str) -> Option<&BitmapFont> {
        let key = Self::cache_key(font_name);
        self.font_cache.get(&key).map(|v| &**v)
    }

    /// Rasterize a specific font member's PFR data at the given size.
    /// Unlike `get_font_with_cast_and_bitmap`, this does NOT search by name —
    /// it uses the provided PFR data directly, ensuring each member gets its own result.
    pub fn rasterize_pfr_at_size(
        &mut self,
        pfr_data: &[u8],
        pfr_parsed: &crate::director::chunks::pfr1::types::Pfr1ParsedFont,
        font_name: &str,
        font_style: u8,
        size: u16,
        bitmap_manager: &mut crate::player::bitmap::manager::BitmapManager,
    ) -> Option<Rc<BitmapFont>> {
        use crate::director::chunks::pfr1::{rasterizer, parse_pfr1_font_with_target};

        let parsed_for_size = match parse_pfr1_font_with_target(pfr_data, size as i32) {
            Ok(p) => p,
            Err(_) => pfr_parsed.clone(),
        };

        let rasterized = rasterizer::rasterize_pfr1_font(&parsed_for_size, size as usize, 0);

        let bitmap_width = rasterized.bitmap_width as u16;
        let bitmap_height = rasterized.bitmap_height as u16;

        let mut bitmap = Bitmap::new(
            bitmap_width, bitmap_height, 32, 32, 0,
            PaletteRef::BuiltIn(get_system_default_palette()),
        );

        let data_len = rasterized.bitmap_data.len().min(bitmap.data.len());
        bitmap.data[..data_len].copy_from_slice(&rasterized.bitmap_data[..data_len]);

        for i in (0..data_len).step_by(4) {
            let a = bitmap.data[i + 3];
            if a == 0 {
                bitmap.data[i] = 255;
                bitmap.data[i + 1] = 255;
                bitmap.data[i + 2] = 255;
            }
        }
        bitmap.use_alpha = true;

        let bitmap_ref = bitmap_manager.add_bitmap(bitmap);

        let font = BitmapFont {
            bitmap_ref,
            char_width: rasterized.cell_width as u16,
            char_height: rasterized.cell_height as u16,
            grid_columns: rasterized.grid_columns as u8,
            grid_rows: rasterized.grid_rows as u8,
            grid_cell_width: rasterized.cell_width as u16,
            grid_cell_height: rasterized.cell_height as u16,
            char_offset_x: 0,
            char_offset_y: 0,
            first_char_num: rasterized.first_char,
            font_name: font_name.to_string(),
            font_size: size,
            font_style,
            char_widths: Some(rasterized.char_widths),
            pfr_native_size: 0,
        };

        Some(Rc::new(font))
    }
}

pub async fn player_load_system_font(path: &str) {
    let window = web_sys::window().unwrap();
    let result = JsFuture::from(window.fetch_with_str(path)).await;

    match result {
        Ok(result) => {
            let result = result.dyn_into::<web_sys::Response>().unwrap();
            let blob = JsFuture::from(result.blob().unwrap()).await.unwrap();
            let blob = blob.dyn_into::<web_sys::Blob>().unwrap();
            let image_data = window.create_image_bitmap_with_blob(&blob).unwrap();
            let image_data = JsFuture::from(image_data).await.unwrap();
            let image_bitmap = image_data.dyn_into::<web_sys::ImageBitmap>().unwrap();

            let canvas = web_sys::window()
                .unwrap()
                .document()
                .unwrap()
                .create_element("canvas")
                .unwrap();
            let canvas = canvas.dyn_into::<web_sys::HtmlCanvasElement>().unwrap();
            canvas.set_width(image_bitmap.width());
            canvas.set_height(image_bitmap.height());
            let context = canvas
                .get_context("2d")
                .unwrap()
                .unwrap()
                .dyn_into::<web_sys::CanvasRenderingContext2d>()
                .unwrap();

            context
                .draw_image_with_image_bitmap(&image_bitmap, 0.0, 0.0)
                .unwrap();

            let image_data = context
                .get_image_data(
                    0.0,
                    0.0,
                    image_bitmap.width() as f64,
                    image_bitmap.height() as f64,
                )
                .unwrap();

            let bitmap = Bitmap {
                width: image_data.width() as u16,
                height: image_data.height() as u16,
                data: image_data.data().0,
                bit_depth: 32,
                original_bit_depth: 32,
                palette_ref: PaletteRef::BuiltIn(get_system_default_palette()),
                matte: None,
                use_alpha: false,
                trim_white_space: false,
                was_trimmed: false,
                version: 0,
            };

            reserve_player_mut(|player| {
                let grid_columns = 18;
                let grid_rows = 7;
                let grid_cell_width = bitmap.width / grid_columns;
                let grid_cell_height = bitmap.height / grid_rows;

                let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                let font = BitmapFont {
                    bitmap_ref,
                    char_width: 5,
                    char_height: 7,
                    grid_columns: grid_columns as u8,
                    grid_rows: grid_rows as u8,
                    grid_cell_width,
                    grid_cell_height,
                    first_char_num: 32,
                    char_offset_x: 1,
                    char_offset_y: 1,
                    font_name: "System".to_string(),
                    font_size: 12,
                    font_style: 0,
                    char_widths: None,
                    pfr_native_size: 0,
                };

                let rc_font = Rc::new(font.clone());

                let font_ref = player.font_manager.font_counter;
                player.font_manager.font_counter += 1;
                player
                    .font_manager
                    .fonts
                    .insert(font_ref, Rc::clone(&rc_font));
                player.font_manager.system_font = Some(rc_font);

                // Add to font_cache where rendering code looks for it
                player
                    .font_manager
                    .font_cache
                    .insert("system".to_string(), font.into());

                debug!("System font loaded successfully");
            });

            warn!("Loaded system font image data: {:?}", image_data);
        }
        Err(err) => {
            warn!("Error fetching system font: {:?}", err);
            return;
        }
    };
}

pub fn bitmap_font_copy_char(
    font: &BitmapFont,
    font_bitmap: &Bitmap,
    char_num: u8,
    dest: &mut Bitmap,
    dest_x: i32,
    dest_y: i32,
    palettes: &PaletteMap,
    draw_params: &CopyPixelsParams,
) {
    // Skip if character is below the font's first character
    if char_num < font.first_char_num {
        return;
    }
    let char_index = (char_num - font.first_char_num) as usize;

    // Calculate grid position
    let char_x = (char_index % font.grid_columns as usize) as u16;
    let char_y = (char_index / font.grid_columns as usize) as u16;

    // Calculate source rectangle in the font bitmap
    let src_x = (char_x * font.grid_cell_width + font.char_offset_x) as i32;
    let src_y = (char_y * font.grid_cell_height + font.char_offset_y) as i32;

    dest.copy_pixels_with_params(
        palettes,
        font_bitmap,
        IntRect::from(
            dest_x,
            dest_y,
            dest_x + font.char_width as i32,
            dest_y + font.char_height as i32,
        ),
        IntRect::from(
            src_x,
            src_y,
            src_x + font.char_width as i32,
            src_y + font.char_height as i32,
        ),
        &draw_params,
    )
}

/// Copy a character using native glyph size but clipped to `clip_width`.
/// Useful for proportional bitmap fonts with very large cell widths.
pub fn bitmap_font_copy_char_clipped(
    font: &BitmapFont,
    font_bitmap: &Bitmap,
    char_num: u8,
    dest: &mut Bitmap,
    dest_x: i32,
    dest_y: i32,
    clip_width: i32,
    palettes: &PaletteMap,
    draw_params: &CopyPixelsParams,
) {
    if char_num < font.first_char_num {
        return;
    }
    let char_index = (char_num - font.first_char_num) as usize;
    let char_x = (char_index % font.grid_columns as usize) as u16;
    let char_y = (char_index / font.grid_columns as usize) as u16;
    let src_x = (char_x * font.grid_cell_width + font.char_offset_x) as i32;
    let src_y = (char_y * font.grid_cell_height + font.char_offset_y) as i32;
    let w = clip_width.max(1).min(font.char_width as i32);

    dest.copy_pixels_with_params(
        palettes,
        font_bitmap,
        IntRect::from(dest_x, dest_y, dest_x + w, dest_y + font.char_height as i32),
        IntRect::from(src_x, src_y, src_x + w, src_y + font.char_height as i32),
        draw_params,
    )
}

/// Copy a character using native glyph size but clipped to `clip_width`,
/// taking the clip from the center of the source cell to preserve glyph ink
/// that may not start at x=0 in very wide PFR cells.
pub fn bitmap_font_copy_char_center_clipped(
    font: &BitmapFont,
    font_bitmap: &Bitmap,
    char_num: u8,
    dest: &mut Bitmap,
    dest_x: i32,
    dest_y: i32,
    clip_width: i32,
    palettes: &PaletteMap,
    draw_params: &CopyPixelsParams,
) {
    if char_num < font.first_char_num {
        return;
    }
    let char_index = (char_num - font.first_char_num) as usize;
    let char_x = (char_index % font.grid_columns as usize) as u16;
    let char_y = (char_index / font.grid_columns as usize) as u16;
    let src_x_base = (char_x * font.grid_cell_width + font.char_offset_x) as i32;
    let src_y = (char_y * font.grid_cell_height + font.char_offset_y) as i32;
    let full_w = font.char_width as i32;
    let w = clip_width.max(1).min(full_w);
    let src_x = src_x_base + ((full_w - w) / 2).max(0);

    dest.copy_pixels_with_params(
        palettes,
        font_bitmap,
        IntRect::from(dest_x, dest_y, dest_x + w, dest_y + font.char_height as i32),
        IntRect::from(src_x, src_y, src_x + w, src_y + font.char_height as i32),
        draw_params,
    )
}

/// Copy a character using native glyph size but clipped to `clip_width`,
/// taking the right-most portion of the source cell.
/// Some imported PFR bitmaps place visible ink toward the right side of wide cells.
pub fn bitmap_font_copy_char_right_clipped(
    font: &BitmapFont,
    font_bitmap: &Bitmap,
    char_num: u8,
    dest: &mut Bitmap,
    dest_x: i32,
    dest_y: i32,
    clip_width: i32,
    palettes: &PaletteMap,
    draw_params: &CopyPixelsParams,
) {
    if char_num < font.first_char_num {
        return;
    }
    let char_index = (char_num - font.first_char_num) as usize;
    let char_x = (char_index % font.grid_columns as usize) as u16;
    let char_y = (char_index / font.grid_columns as usize) as u16;
    let src_x_base = (char_x * font.grid_cell_width + font.char_offset_x) as i32;
    let src_y = (char_y * font.grid_cell_height + font.char_offset_y) as i32;
    let full_w = font.char_width as i32;
    let w = clip_width.max(1).min(full_w);
    let src_x = src_x_base + (full_w - w).max(0);

    dest.copy_pixels_with_params(
        palettes,
        font_bitmap,
        IntRect::from(dest_x, dest_y, dest_x + w, dest_y + font.char_height as i32),
        IntRect::from(src_x, src_y, src_x + w, src_y + font.char_height as i32),
        draw_params,
    )
}

/// Copy a character from a bitmap font to a destination bitmap with scaling
/// The scale factor determines the output size relative to the font's native size
pub fn bitmap_font_copy_char_scaled(
    font: &BitmapFont,
    font_bitmap: &Bitmap,
    char_num: u8,
    dest: &mut Bitmap,
    dest_x: i32,
    dest_y: i32,
    dest_char_width: i32,
    dest_char_height: i32,
    palettes: &PaletteMap,
    draw_params: &CopyPixelsParams,
) {
    // Skip if character is below the font's first character
    if char_num < font.first_char_num {
        return;
    }
    let char_index = (char_num - font.first_char_num) as usize;

    // Calculate grid position
    let char_x = (char_index % font.grid_columns as usize) as u16;
    let char_y = (char_index / font.grid_columns as usize) as u16;

    // Calculate source rectangle in the font bitmap (native size)
    let src_x = (char_x * font.grid_cell_width + font.char_offset_x) as i32;
    let src_y = (char_y * font.grid_cell_height + font.char_offset_y) as i32;

    // Copy with scaling - dest rect is scaled, src rect is native font size
    dest.copy_pixels_with_params(
        palettes,
        font_bitmap,
        IntRect::from(
            dest_x,
            dest_y,
            dest_x + dest_char_width,
            dest_y + dest_char_height,
        ),
        IntRect::from(
            src_x,
            src_y,
            src_x + font.char_width as i32,
            src_y + font.char_height as i32,
        ),
        &draw_params,
    )
}

/// Copy a character using a tight source rectangle computed from non-white ink pixels.
/// This is useful for fonts imported into very wide cells where advance is much smaller
/// than the cell width.
pub fn bitmap_font_copy_char_tight(
    font: &BitmapFont,
    font_bitmap: &Bitmap,
    char_num: u8,
    dest: &mut Bitmap,
    dest_x: i32,
    dest_y: i32,
    palettes: &PaletteMap,
    draw_params: &CopyPixelsParams,
) {
    if char_num < font.first_char_num {
        return;
    }
    let char_index = (char_num - font.first_char_num) as usize;
    let char_x = (char_index % font.grid_columns as usize) as u16;
    let char_y = (char_index / font.grid_columns as usize) as u16;
    let src_x_base = (char_x * font.grid_cell_width + font.char_offset_x) as i32;
    let src_y_base = (char_y * font.grid_cell_height + font.char_offset_y) as i32;
    let full_w = font.char_width as i32;
    let full_h = font.char_height as i32;

    let bmp_w = font_bitmap.width as i32;
    let bmp_h = font_bitmap.height as i32;

    let mut min_x = full_w;
    let mut max_x = -1;
    let mut min_y = full_h;
    let mut max_y = -1;

    for y in 0..full_h {
        let sy = src_y_base + y;
        if sy < 0 || sy >= bmp_h {
            continue;
        }
        for x in 0..full_w {
            let sx = src_x_base + x;
            if sx < 0 || sx >= bmp_w {
                continue;
            }
            let idx = ((sy * bmp_w + sx) * 4) as usize;
            if idx + 3 >= font_bitmap.data.len() {
                continue;
            }
            let r = font_bitmap.data[idx];
            let g = font_bitmap.data[idx + 1];
            let b = font_bitmap.data[idx + 2];
            let a = font_bitmap.data[idx + 3];
            // Treat any non-white source pixel as ink for tight bounds.
            // Some PFR atlases store visible strokes with weak/zero alpha.
            if !(r >= 250 && g >= 250 && b >= 250) {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    if max_x < min_x || max_y < min_y {
        // No tight bounds found; fall back to normal copy.
        bitmap_font_copy_char(
            font,
            font_bitmap,
            char_num,
            dest,
            dest_x,
            dest_y,
            palettes,
            draw_params,
        );
        return;
    }

    let src_x = src_x_base + min_x;
    let src_y = src_y_base + min_y;
    let w = (max_x - min_x + 1).max(1);
    let h = (max_y - min_y + 1).max(1);

    dest.copy_pixels_with_params(
        palettes,
        font_bitmap,
        IntRect::from(dest_x + min_x, dest_y + min_y, dest_x + min_x + w, dest_y + min_y + h),
        IntRect::from(src_x, src_y, src_x + w, src_y + h),
        draw_params,
    )
}

pub fn measure_text(
    text: &str,
    font: &BitmapFont,
    line_height: Option<u16>,
    line_spacing: u16,
    top_spacing: i16,
    bottom_spacing: i16,
) -> (u16, u16) {
    let mut width = 0;
    let mut line_width = 0;
    // PFR bitmap fonts render at native char_height (no scaling).
    // Use char_height - 1 to match Shockwave's line height for PFR fonts.
    let effective_line_h = if font.char_widths.is_some() {
        font.char_height.saturating_sub(1)
    } else if font.font_size > 0 {
        font.font_size
    } else {
        font.char_height
    };
    let line_height = line_height.unwrap_or(effective_line_h);
    // fixedLineSpace overrides line step between lines; topSpacing + bottomSpacing added on top.
    let effective_lh = if line_spacing > 0 { line_spacing as i16 } else { line_height as i16 };
    // First line uses the max of font height and line spacing so glyphs aren't clipped,
    // but the field's STXT line height is also respected when it's larger than the font.
    let first_line_h = (line_height as i16).max(effective_lh);
    let mut height = (top_spacing + first_line_h) as u16;
    let line_step = (effective_lh + bottom_spacing + top_spacing) as u16;
    let mut index = 0;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if line_width > width {
                width = line_width;
            }
            line_width = 0;
        } else {
            if line_width == 0 && index > 0 {
                height += line_step;
            }
            let adv = font.get_char_advance(c as u8);
            if font.char_widths.is_some() {
                line_width += adv;
            } else {
                line_width += adv + 1;
            }
        }
        index += 1;
    }
    if line_width > width {
        width = line_width;
    }
    return (width, height);
}

/// Measure text height with word wrapping support.
/// Returns (max_line_width, total_height) considering word wrapping at max_width.
pub fn measure_text_wrapped(
    text: &str,
    font: &BitmapFont,
    max_width: u16,
    word_wrap: bool,
    line_spacing: u16,
    top_spacing: i16,
    bottom_spacing: i16,
) -> (u16, u16) {
    let effective_line_h = if font.char_widths.is_some() {
        font.char_height.saturating_sub(1)
    } else if font.font_size > 0 {
        font.font_size
    } else {
        font.char_height
    };
    let effective_lh = if line_spacing > 0 { line_spacing as i16 } else { effective_line_h as i16 };
    let line_step = (effective_lh + bottom_spacing + top_spacing) as u16;

    // Split into explicit lines first
    let raw_lines: Vec<&str> = text.split(|c: char| c == '\r' || c == '\n').collect();
    let mut visual_lines: Vec<u16> = Vec::new(); // width of each visual line

    for raw in &raw_lines {
        if raw.is_empty() {
            visual_lines.push(0);
            continue;
        }
        if word_wrap && max_width > 0 {
            let mut current_width: u16 = 0;
            for word in raw.split_whitespace() {
                let word_width: u16 = word.chars()
                    .map(|c| font.get_char_advance(c as u8))
                    .sum();
                let space_width = font.get_char_advance(b' ');
                let candidate = if current_width == 0 {
                    word_width
                } else {
                    current_width + space_width + word_width
                };
                if candidate <= max_width || current_width == 0 {
                    current_width = candidate;
                } else {
                    visual_lines.push(current_width);
                    current_width = word_width;
                }
            }
            visual_lines.push(current_width);
        } else {
            let line_width: u16 = raw.chars()
                .map(|c| font.get_char_advance(c as u8))
                .sum();
            visual_lines.push(line_width);
        }
    }

    let num_lines = visual_lines.len().max(1);
    let max_width_found = visual_lines.iter().copied().max().unwrap_or(0);
    let first_line_h = (effective_line_h as i16).max(effective_lh);
    let height = (top_spacing + first_line_h) as u16
        + (num_lines as u16 - 1) * line_step;

    (max_width_found, height)
}

pub fn get_text_char_pos(text: &str, params: &DrawTextParams, char_index: usize) -> (i16, i16) {
    let mut x = 0;
    let mut y = params.top_spacing;
    let mut line_width = 0;
    let mut line_index = 0;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if line_index == char_index {
                return (x, y);
            }
            if line_width > x {
                x = line_width;
            }
            line_width = 0;
            let eff_lh = if params.font.font_size > 0 { params.font.font_size } else { params.font.char_height };
            y += params.line_height.unwrap_or(eff_lh) as i16
                + params.line_spacing as i16
                + 1;
        } else {
            if line_index == char_index {
                return (x, y);
            }
            line_width += params.font.get_char_advance(c as u8) as i16 + 1;
        }
        line_index += 1;
    }
    if line_width > x {
        x = line_width;
    }
    return (x, y);
}

pub fn get_text_index_at_pos(text: &str, params: &DrawTextParams, x: i32, y: i32) -> usize {
    let eff_lh = if params.font.font_size > 0 { params.font.font_size } else { params.font.char_height };
    let line_h = params.line_height.unwrap_or(eff_lh) as i32;
    let mut index = 0;
    let mut line_width = 0;
    let mut line_y = params.top_spacing as i32;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if y >= line_y && y < line_y + line_h {
                if x < line_width {
                    return index;
                }
            }
            if line_width > x {
                line_width = 0;
            }
            line_y += line_h + params.line_spacing as i32 + 1;
        } else {
            if y >= line_y && y < line_y + line_h {
                if x < line_width {
                    return index;
                }
            }
            line_width += params.font.get_char_advance(c as u8) as i32 + 1;
        }
        index += 1;
    }
    return index;
}
