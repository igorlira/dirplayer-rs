use fxhash::FxHashMap;
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
    pub pfr_enabled: bool,
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

    /// Advance width for a Unicode char. Maps the char to its Win-1252
    /// glyph slot first, so codepoints like € (U+20AC) hit the correct
    /// atlas cell (0x80) instead of `c as u8` truncating to 0xAC.
    #[inline]
    pub fn get_char_advance_for(&self, c: char) -> u16 {
        self.get_char_advance(crate::io::encoding::glyph_byte_for(c))
    }
}

pub struct DrawTextParams<'a> {
    pub font: &'a BitmapFont,
    pub line_height: Option<u16>,
    pub line_spacing: u16,
    pub top_spacing: i16,
    pub char_spacing: i16,
    pub member_width: Option<i16>,
    /// Minimum advance for the space character. When > 0, any stored
    /// space advance below this is clamped up to this value during both
    /// wrap measurement AND char-index calculation. Mirrors the renderer's
    /// `space_min_advance` clamp so hit-testing and drawing agree on
    /// which char is at a given pixel position. Without this, fields
    /// drawn with widened spaces but measured with raw narrow spaces
    /// would have `the mouseChar` return positions in the wrong run
    /// (clicks on "Christ's passion" hitting "the sign of the cross").
    pub min_space_advance: Option<i16>,
    /// Pre-computed per-character advances (one entry per char of the
    /// text, including newlines — those entries are ignored). When
    /// `Some`, these are used instead of `font.get_char_advance(c)` —
    /// callers can plug in run-aware advances (e.g. Arial Bold widths
    /// for an underlined run on top of an otherwise-Arial field) so
    /// the hit-test wraps with the same per-run metrics the renderer
    /// draws with. Length must equal `text.chars().count()`; out-of-range
    /// lookups fall back to the font's advance. The space-min clamp and
    /// `char_spacing` are NOT re-applied on top of these — the caller
    /// must bake them in if needed.
    pub per_char_advances: Option<&'a [i32]>,
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
        // Match priority — needed because PFR1 internal `font_info.name`
        // uses the convention `<FamilyName>_<Weight>_<Other>` (e.g.
        // "Arial_700_000" for Bold, "Arial_400_000" for Regular). Our
        // canonical_font_name strips trailing digit-only tokens, so
        // ALL Arial variants canonicalize via info_name to plain
        // "arial" — meaning a request for "Arial *" used to match
        // whichever variant iterated first (often Bold), and the body
        // of every field rendered bold. Member names ("Arial *",
        // "Arial Bold *", "Arial Italic *") DO preserve the variant
        // distinction after canonicalization, so we match on those
        // first and only fall back to info_name canonical when no
        // member-name match exists.
        //   tier 0: exact lowercase match on member.name or info_name
        //   tier 1: canonical member.name match
        //   tier 2: canonical info_name match (fallback)
        let mut best_tier: Option<u8> = None;
        let mut best: Option<(u32, u32, u32)> = None; // (cast_lib_num, member_id, tier)
        for cast_lib in &cast_manager.casts {
            for (&member_id, member) in cast_lib.members.iter() {
                if let CastMemberType::Font(font_data) = &member.member_type {
                    let info_name_canon = Self::canonical_font_name(&font_data.font_info.name);
                    let member_name_canon = Self::canonical_font_name(&member.name);
                    let exact_match = font_data.font_info.name.to_lowercase() == font_name_lc
                        || member.name.to_lowercase() == font_name_lc;
                    let member_canon_match = !font_name_canon.is_empty()
                        && member_name_canon == font_name_canon;
                    let info_canon_match = !font_name_canon.is_empty()
                        && info_name_canon == font_name_canon;
                    let tier: Option<u8> = if exact_match {
                        Some(0)
                    } else if member_canon_match {
                        Some(1)
                    } else if info_canon_match {
                        Some(2)
                    } else {
                        None
                    };
                    if let Some(t) = tier {
                        if best_tier.map_or(true, |bt| t < bt) {
                            best_tier = Some(t);
                            best = Some((cast_lib.number, member_id, t as u32));
                        }
                    }
                }
            }
        }
        // Walk the loop again only when we have a winner, to actually
        // rasterize and cache it. (Re-borrows are fine; the priority
        // pass above is read-only.)
        let winner = best;
        for cast_lib in &cast_manager.casts {
            for (&member_id, member) in cast_lib.members.iter() {
                if winner.map_or(true, |(cl, mid, _)| cl != cast_lib.number || mid != member_id) {
                    continue;
                }
                if let CastMemberType::Font(font_data) = &member.member_type {
                    if let Some(ref parsed) = font_data.pfr_parsed {
                        use crate::director::chunks::pfr1::{rasterizer, parse_pfr1_font_with_target};

                        // Match FontinatorFINAL's working approach: parse with
                        // target_em_px = outline_resolution rather than 0 or
                        // the small render size. Hinting still fires, but at
                        // ~unity scale (the multiplier `target/outline_res` is
                        // 1), so it doesn't try to snap stems into a 12-pixel
                        // box and fragment glyphs. The rasterizer then handles
                        // the actual downscale to the displayed size via
                        // `scale = target_height / target_em_px` in the
                        // coords_scaled branch. Fontinator's 9px atlas output
                        // for fugue_arial / fugue_arial_italic verifies this
                        // produces complete, thin glyphs.
                        let outline_res = parsed.physical_font.outline_resolution as i32;
                        let parse_target = if outline_res > 0 { outline_res } else { 0 };
                        let parsed_for_size = if let Some(ref raw) = font_data.pfr_data {
                            match parse_pfr1_font_with_target(raw, parse_target) {
                                Ok(p) => p,
                                Err(_) => parsed.clone(),
                            }
                        } else {
                            parsed.clone()
                        };

                        // Enable the thin-stem alpha boost only for italic
                        // variants. Italic glyphs concentrate the 1-orus-
                        // wide vertical stems (lowercase l, i, uppercase I)
                        // whose AA coverage at small sizes is too faint
                        // without the sqrt() gamma lift. Regular and bold
                        // atlases have wider strokes — the boost would
                        // thicken their AA edges and make the whole atlas
                        // look bolder than Shockwave's reference render.
                        let font_info_lc = font_data.font_info.name.to_ascii_lowercase();
                        let member_name_lc = member.name.to_ascii_lowercase();
                        let thin_stem_boost = font_info_lc.contains("italic")
                            || member_name_lc.contains("italic");
                        let rasterized = rasterizer::rasterize_pfr1_font_with_options(
                            &parsed_for_size,
                            requested_size as usize,
                            font_data.font_info.size as usize,
                            thin_stem_boost,
                        );

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

                    // Match the cast-member PFR re-parse path above:
                    // parse with target_em_px = outline_resolution so hinting
                    // fires at unity scale, then let the rasterizer downscale.
                    // First parse with target=0 to learn outline_res, then
                    // re-parse with that as target.
                    let parsed_for_res = match parse_pfr1_font_with_target(pfr_bytes, 0) {
                        Ok(p) => Some(p),
                        Err(_) => None,
                    };
                    let parse_target = parsed_for_res.as_ref()
                        .map(|p| p.physical_font.outline_resolution as i32)
                        .filter(|&v| v > 0)
                        .unwrap_or(0);
                    match parse_pfr1_font_with_target(pfr_bytes, parse_target) {
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
            pfr_enabled: true,
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
            let result = if let Some(blob) = result.dyn_ref::<web_sys::Blob>() {
                web_sys::Response::new_with_opt_blob(Some(blob)).unwrap()
            } else {
                result.dyn_into::<web_sys::Response>().unwrap()
            };
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

/// Italic shear factor: each row is shifted right by
/// `(glyph_height - 1 - row) / ITALIC_SHEAR_DIVISOR`. Smaller divisor → steeper
/// slant. 4 matches Director's emulated italic on bitmap fonts at typical UI
/// sizes (12–16px), giving 3–4px of total shear from baseline to ascender.
const ITALIC_SHEAR_DIVISOR: i32 = 4;

/// Return the horizontal italic shear (in pixels) needed to add to the right
/// edge of a glyph cell so the slanted top doesn't visually collide with the
/// next character. Callers that build per-glyph cursor advance use this to
/// reserve room for the slant.
pub fn italic_shear_for_height(char_height: i32) -> i32 {
    (char_height.max(1) - 1) / ITALIC_SHEAR_DIVISOR
}

/// Like `bitmap_font_copy_char` but applies a per-row horizontal shear to fake
/// italic on a non-italic bitmap font. Source rows are copied 1px-tall at a
/// time so each scanline can land at its own dest_x. PFR atlases are pre-
/// rasterized — we cannot get a real italic face from the bitmap, but per-row
/// shear gives the slanted look that Director produces when fontStyle includes
/// `#italic` and no italic face is available.
pub fn bitmap_font_copy_char_italic(
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
    let src_x = (char_x * font.grid_cell_width + font.char_offset_x) as i32;
    let src_y = (char_y * font.grid_cell_height + font.char_offset_y) as i32;
    let w = font.char_width as i32;
    let h = font.char_height as i32;

    for row in 0..h {
        let shift = (h - 1 - row) / ITALIC_SHEAR_DIVISOR;
        dest.copy_pixels_with_params(
            palettes,
            font_bitmap,
            IntRect::from(
                dest_x + shift,
                dest_y + row,
                dest_x + shift + w,
                dest_y + row + 1,
            ),
            IntRect::from(
                src_x,
                src_y + row,
                src_x + w,
                src_y + row + 1,
            ),
            draw_params,
        );
    }
}

/// Tight + italic: combines `bitmap_font_copy_char_tight`'s ink-bounds trim
/// with per-row shear. For each row of the trimmed bbox, copy 1px tall at the
/// sheared dest_x. Fall back to non-tight italic when the cell has no ink.
pub fn bitmap_font_copy_char_tight_italic(
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
            if !(r >= 250 && g >= 250 && b >= 250) {
                min_x = min_x.min(x);
                max_x = max_x.max(x);
                min_y = min_y.min(y);
                max_y = max_y.max(y);
            }
        }
    }

    if max_x < min_x || max_y < min_y {
        bitmap_font_copy_char_italic(
            font, font_bitmap, char_num, dest, dest_x, dest_y, palettes, draw_params,
        );
        return;
    }

    let src_x = src_x_base + min_x;
    let src_y_top = src_y_base + min_y;
    let w = (max_x - min_x + 1).max(1);
    let h = (max_y - min_y + 1).max(1);

    for row in 0..h {
        // Shear is computed against full glyph height so the slant is consistent
        // across glyphs — using only the trimmed `h` would give wider chars more
        // slant than narrow ones at the same point size.
        let global_row = min_y + row;
        let shift = (full_h - 1 - global_row) / ITALIC_SHEAR_DIVISOR;
        dest.copy_pixels_with_params(
            palettes,
            font_bitmap,
            IntRect::from(
                dest_x + min_x + shift,
                dest_y + min_y + row,
                dest_x + min_x + shift + w,
                dest_y + min_y + row + 1,
            ),
            IntRect::from(
                src_x,
                src_y_top + row,
                src_x + w,
                src_y_top + row + 1,
            ),
            draw_params,
        );
    }
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
    // Use char_height - 1 to match Shockwave's line height — but cap at
    // `font_size × 1.5` to handle tiny pixel fonts (04b_08 *) whose atlas
    // pads the cell to ~2× the nominal size. Without the cap, member 16's
    // 19 lines of 04b_08 * render at char_height-1=25 px each (485 px
    // total) instead of Director's ~20 px (375 px). Tight-cell PFR fonts
    // (Verdana/Arial 12pt: char_height≈14, cell-1=13) keep the smaller
    // value and don't regress.
    let effective_line_h = if font.char_widths.is_some() {
        let cell_h = font.char_height.saturating_sub(1);
        if font.font_size > 0 {
            let cap = ((font.font_size as f32) * 1.5).round() as u16;
            cell_h.min(cap)
        } else {
            cell_h
        }
    } else if font.font_size > 0 {
        font.font_size
    } else {
        font.char_height
    };
    let line_height = line_height.unwrap_or(effective_line_h);
    // Same safety guard the renderer applies (see webgl2/mod.rs): treat
    // fixed_line_space values much larger than the natural line height as
    // XMED-misparsed field heights and fall back. Without this, the field's
    // auto-grown sprite_rect inflates to many times its rendered height,
    // which expands the I-beam hit-test area into empty space below the
    // actual text.
    let natural_lh_for_guard = if font.font_size > 0 {
        font.font_size
    } else {
        font.char_height
    };
    let line_spacing = if line_spacing > 0
        && (line_spacing as u32) > (natural_lh_for_guard as u32 * 5 / 2)
    {
        0
    } else {
        line_spacing
    };
    // fixedLineSpace overrides line step between lines; topSpacing + bottomSpacing added on top.
    let effective_lh = if line_spacing > 0 { line_spacing as i16 } else { line_height as i16 };
    // First line height: when an explicit line_spacing (member's
    // fixedLineSpace) is set, trust it verbatim — that's the authored
    // per-line stride and Director uses it as the first-line extent too.
    // Without the gate, `cell_h = char_height - 1` for PFR fonts gives
    // exactly `fixed_line_space + 1` (e.g. 22 for fixed_line_space=21),
    // which inflates `member.height` by N px on N-line lists. Junkbot's
    // level-name member with fixed_line_space=21 reported height=22 for
    // one rendered line when the authored 16-paragraph layout was 331.
    // Fall back to `max(line_height, effective_lh)` for members without
    // explicit line_spacing so glyphs still aren't clipped.
    let first_line_h = if line_spacing > 0 {
        effective_lh
    } else {
        (line_height as i16).max(effective_lh)
    };
    // Guard against negative top_spacing (set by the field renderer to
    // implement scrollTop via a negative offset). The original formulas
    // `(top_spacing + first_line_h) as u16` and
    // `(effective_lh + bottom_spacing + top_spacing) as u16` underflow
    // when top_spacing is negative, producing huge line_step values that
    // cause `(num_lines-1) * line_step` to balloon to ~50M for paged
    // fields — and the resulting bitmap allocation either fails or hangs,
    // leaving the field stuck at whatever the last successful scroll
    // position was. Clamp via i32 arithmetic and saturate at the bottom.
    let mut height = ((top_spacing as i32) + (first_line_h as i32)).max(0) as u16;
    let line_step = ((effective_lh as i32) + (bottom_spacing as i32) + (top_spacing.max(0) as i32)).max(1) as u16;
    let mut index = 0;
    let mut last_was_newline = false;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if line_width > width {
                width = line_width;
            }
            line_width = 0;
            last_was_newline = true;
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
            last_was_newline = false;
        }
        index += 1;
    }
    if line_width > width {
        width = line_width;
    }
    // Account for a trailing empty line. Without this, text like "abc\r"
    // measures the same as "abc", so an editable field that just received
    // an Enter keystroke doesn't grow vertically — the renderer happily
    // draws the new (empty) line at y = line_step but the sprite_rect
    // stays one-line tall, pushing the caret outside the visible field box.
    if last_was_newline {
        height += line_step;
    }
    return (width, height);
}

/// Measure text height with word wrapping support.
/// Returns (max_line_width, total_height) considering word wrapping at max_width.
/// `char_spacing` is added between every pair of characters (matches renderer's behavior).
pub fn measure_text_wrapped(
    text: &str,
    font: &BitmapFont,
    max_width: u16,
    word_wrap: bool,
    line_spacing: u16,
    top_spacing: i16,
    bottom_spacing: i16,
    char_spacing: i32,
) -> (u16, u16) {
    // See `measure_text` for the cap rationale — PFR pixel fonts pad
    // the atlas cell to ~2× the nominal size, so we cap at `font_size × 1.5`.
    let effective_line_h = if font.char_widths.is_some() {
        let cell_h = font.char_height.saturating_sub(1);
        if font.font_size > 0 {
            let cap = ((font.font_size as f32) * 1.5).round() as u16;
            cell_h.min(cap)
        } else {
            cell_h
        }
    } else if font.font_size > 0 {
        font.font_size
    } else {
        font.char_height
    };
    // Same safety guard as the renderer / measure_text — drop misset
    // fixed_line_space values that look like field heights so the auto-
    // grown sprite_rect doesn't inflate the hit-test area past the
    // visible text.
    let natural_lh_for_guard = if font.font_size > 0 {
        font.font_size
    } else {
        font.char_height
    };
    let line_spacing = if line_spacing > 0
        && (line_spacing as u32) > (natural_lh_for_guard as u32 * 5 / 2)
    {
        0
    } else {
        line_spacing
    };
    let effective_lh = if line_spacing > 0 { line_spacing as i16 } else { effective_line_h as i16 };
    // Mirror the i32-clamped formula in measure_text — negative top_spacing
    // (used by the field renderer for scrollTop) would otherwise underflow
    // here and break paged-field rendering past ~font_size px.
    let line_step = ((effective_lh as i32) + (bottom_spacing as i32) + (top_spacing.max(0) as i32)).max(1) as u16;

    // Per-character advance including char_spacing — matches the renderer at
    // text.rs's flush_line loop which does `x += adv + char_spacing` for every char.
    let char_adv = |c: char| -> i32 {
        font.get_char_advance(c as u8) as i32 + char_spacing
    };

    // Split into explicit lines first
    let raw_lines: Vec<&str> = text.split(|c: char| c == '\r' || c == '\n').collect();
    let mut visual_lines: Vec<i32> = Vec::new(); // width of each visual line

    for raw in &raw_lines {
        if raw.is_empty() {
            visual_lines.push(0);
            continue;
        }
        if word_wrap && max_width > 0 {
            // Mirror wrap_lines_with_spans exactly: character-level processing
            // with space-breaks preferred and hard-breaks for overlong words.
            // Using the same algorithm ensures measure and render agree on line
            // count, so the sprite rect is always tall enough for what renders.
            let max_w = max_width as i32;
            let raw_bytes = raw.as_bytes();
            let rlen = raw_bytes.len();
            let mut line_w: i32 = 0;
            let mut line_start = 0usize;
            let mut last_space: Option<usize> = None;
            let mut p = 0usize;
            while p < rlen {
                let b = raw_bytes[p];
                let cw = char_adv(b as char);
                if b == b' ' {
                    last_space = Some(p);
                }
                if line_w + cw > max_w && p > line_start {
                    if let Some(sp) = last_space.filter(|&sp| sp > line_start) {
                        let w: i32 = raw_bytes[line_start..sp]
                            .iter()
                            .map(|&b| char_adv(b as char))
                            .sum();
                        visual_lines.push(w);
                        line_start = sp + 1;
                        last_space = None;
                        if line_start > p {
                            line_w = 0;
                            p = line_start;
                            continue;
                        }
                        line_w = raw_bytes[line_start..p]
                            .iter()
                            .map(|&b| char_adv(b as char))
                            .sum();
                    } else {
                        #[cfg(feature = "word_hard_break")]
                        {
                            visual_lines.push(line_w);
                            line_start = p;
                            last_space = None;
                            line_w = 0;
                        }
                    }
                }
                line_w += cw;
                p += 1;
            }
            visual_lines.push(line_w);
        } else {
            let line_width: i32 = raw.chars().map(char_adv).sum();
            visual_lines.push(line_width);
        }
    }

    let num_lines = visual_lines.len().max(1);
    let max_width_found = visual_lines.iter().copied().max().unwrap_or(0).max(0) as u16;
    let first_line_h = (effective_line_h as i16).max(effective_lh);
    // Same i32-clamp as measure_text — protects against scrollTop's
    // negative top_spacing pushing bitmap height into the u16 wrap.
    let height_first = ((top_spacing as i32) + (first_line_h as i32)).max(0) as u16;
    let height = height_first + (num_lines as u16 - 1) * line_step;

    (max_width_found, height)
}

pub fn get_text_char_pos(text: &str, params: &DrawTextParams, char_index: usize) -> (i16, i16) {
    let mut x: i16 = 0;
    let mut y = params.top_spacing;
    let mut line_width: i16 = 0;
    let mut line_index = 0;
    let eff_lh = if params.font.font_size > 0 { params.font.font_size } else { params.font.char_height };
    // Match the renderer's line_step (no +1) — see comment in
    // get_text_index_at_pos for the line-by-line drift it causes.
    let line_step = params.line_height.unwrap_or(eff_lh) as i16
        + params.line_spacing as i16;
    let wrap_w = params.member_width.unwrap_or(0) as i32;
    let wrap_enabled = wrap_w > 0;

    // Pre-scan wrap-break positions so the y returned reflects the
    // VISUAL line layout (matches the renderer + locToCharPos), not
    // source-line indices. Critical for AdvanceScroll's
    // `charPosToLoc(member, selStart+1)` — without wrap awareness the
    // returned y misses the visible target line by the number of
    // wrapped visual lines preceding it in long paragraphs.
    // No +1 — matches measure_text_wrapped / get_text_index_at_pos so
    // wrap points line up with the renderer's layout.
    let char_adv_i32 = |c: char, char_idx: usize| -> i32 {
        if let Some(v) = params.per_char_advances {
            if let Some(adv) = v.get(char_idx) {
                return *adv;
            }
        }
        let raw = params.font.get_char_advance(c as u8) as i32;
        let clamped = if c == ' ' {
            raw.max(params.min_space_advance.unwrap_or(0) as i32)
        } else {
            raw
        };
        clamped + params.char_spacing as i32
    };
    let mut wrap_starts: Vec<usize> = Vec::new();
    if wrap_enabled {
        let mut line_w: i32 = 0;
        let mut line_start_byte: usize = 0;
        let mut last_space_after_byte: Option<usize> = None;
        let mut last_space_w: i32 = 0;
        let mut char_idx_pre: usize = 0;
        for (byte_pos, c) in text.char_indices() {
            if c == '\r' || c == '\n' {
                wrap_starts.push(byte_pos + c.len_utf8());
                line_start_byte = byte_pos + c.len_utf8();
                last_space_after_byte = None;
                last_space_w = 0;
                line_w = 0;
                char_idx_pre += 1;
                continue;
            }
            let cw = char_adv_i32(c, char_idx_pre);
            char_idx_pre += 1;
            if line_w + cw > wrap_w && byte_pos > line_start_byte {
                if let Some(sp_after) = last_space_after_byte
                    .filter(|&sp| sp > line_start_byte)
                {
                    wrap_starts.push(sp_after);
                    line_start_byte = sp_after;
                    last_space_after_byte = None;
                    line_w -= last_space_w;
                    last_space_w = 0;
                }
            }
            line_w += cw;
            if c == ' ' {
                last_space_after_byte = Some(byte_pos + 1);
                last_space_w = line_w;
            }
        }
    }

    let mut visual_line_idx = 0usize;
    let mut next_wrap = wrap_starts.first().copied();
    let mut byte_pos = 0usize;
    let mut prev_was_cr = false;
    for c in text.chars() {
        let c_bytes = c.len_utf8();
        // Apply pending wrap-break BEFORE checking char_index for this char
        // so visual line advances when the next char actually crosses a wrap.
        while let Some(wp) = next_wrap {
            if wp == byte_pos && byte_pos != 0 {
                line_width = 0;
                y += line_step;
                visual_line_idx += 1;
                next_wrap = wrap_starts.get(visual_line_idx).copied();
            } else {
                break;
            }
        }
        if c == '\r' || c == '\n' {
            if line_index == char_index {
                return (line_width, y);
            }
            if c == '\n' && prev_was_cr {
                prev_was_cr = false;
                line_index += 1;
                byte_pos += c_bytes;
                continue;
            }
            prev_was_cr = c == '\r';
            line_width = 0;
            y += line_step;
            // Source \r/\n advances a visual line; skip a matching
            // wrap_starts entry if it points to the next byte.
            if let Some(wp) = next_wrap {
                if wp == byte_pos + c_bytes {
                    visual_line_idx += 1;
                    next_wrap = wrap_starts.get(visual_line_idx).copied();
                }
            }
        } else {
            prev_was_cr = false;
            // Honor per_char_advances if provided (caller pre-computed
            // run-aware widths), otherwise fall back to font lookup
            // with the same space-min clamp the renderer uses.
            let char_advance: i16 = if let Some(v) = params
                .per_char_advances
                .and_then(|v| v.get(line_index))
            {
                (*v as i16).max(0)
            } else {
                let raw_adv = params.font.get_char_advance(c as u8) as i16;
                let clamped_adv = if c == ' ' {
                    raw_adv.max(params.min_space_advance.unwrap_or(0))
                } else {
                    raw_adv
                };
                clamped_adv + params.char_spacing
            };
            if line_index == char_index {
                return (line_width, y);
            }
            line_width += char_advance;
        }
        line_index += 1;
        byte_pos += c_bytes;
    }
    if line_width > x {
        x = line_width;
    }
    return (x, y);
}

pub fn get_text_index_at_pos(text: &str, params: &DrawTextParams, x: i32, y: i32) -> usize {
    let eff_lh = if params.font.font_size > 0 { params.font.font_size } else { params.font.char_height };
    let line_h = params.line_height.unwrap_or(eff_lh) as i32;
    // line_step must match the renderer's per-line vertical advance
    // (measure_text_wrapped uses `effective_lh + line_spacing` with no
    // extra +1). Diverging by 1px per line accumulates over wrapped text
    // and shifts hit-test results by an entire line by the time the user
    // clicks on the body of a paragraph — visible as clicks on "Christ's
    // passion" landing on "three motives" or "a crown" instead.
    let line_step = line_h + params.line_spacing as i32;
    let wrap_w = params.member_width.unwrap_or(0) as i32;
    let wrap_enabled = wrap_w > 0;
    // Pre-scan word-wrap break positions so the y-mapping reflects the
    // VISUAL line layout, not the source `\r\n` layout. Without this,
    // a Narrative-style field whose 26k-char body has only a handful of
    // paragraph breaks renders into dozens of wrapped lines, but the
    // hit-test would treat them as one giant single line and clamp y
    // past the bottom — producing `text.length` and breaking the
    // PageNext animation precondition (`locToCharPos(point(1, pageHeight))
    // < text.length`).
    //
    // Each entry is the byte-position of the FIRST char of a visual
    // line. Index 0 is implicit (start of text).
    //
    // Char advance matches measure_text_wrapped (no +1) so wrap points
    // line up with the renderer's actual layout — diverging would cause
    // charPosToLoc and locToCharPos to disagree on which line a char is
    // on, throwing AdvanceScroll off-target by several visual lines.
    let char_adv = |c: char, char_idx: usize| -> i32 {
        if let Some(v) = params.per_char_advances {
            if let Some(adv) = v.get(char_idx) {
                return *adv;
            }
        }
        let raw = params.font.get_char_advance(c as u8) as i32;
        let clamped = if c == ' ' {
            raw.max(params.min_space_advance.unwrap_or(0) as i32)
        } else {
            raw
        };
        clamped + params.char_spacing as i32
    };
    let mut wrap_starts: Vec<usize> = Vec::new();
    if wrap_enabled {
        // Iterate CHARS not bytes — multi-byte chars (ñ, á, ç in
        // Narrative's Spanish/Portuguese top header) would otherwise be
        // counted twice with garbage char_adv per UTF-8 byte, miscounting
        // line widths and producing wrap_starts at wrong byte positions.
        // The main loop matches by byte_pos, so wrong positions cause
        // visual-line jumps in scattered y ranges (the symptom: 271 OK,
        // 272..285 broken, 286 OK).
        let mut line_w: i32 = 0;
        let mut line_start_byte: usize = 0;
        let mut last_space_after_byte: Option<usize> = None; // byte index AFTER the space
        let mut last_space_w: i32 = 0; // line_w at the space (so we can split cleanly)
        let mut char_idx_pre: usize = 0;
        for (byte_pos, c) in text.char_indices() {
            if c == '\r' || c == '\n' {
                wrap_starts.push(byte_pos + c.len_utf8());
                line_start_byte = byte_pos + c.len_utf8();
                last_space_after_byte = None;
                last_space_w = 0;
                line_w = 0;
                char_idx_pre += 1;
                continue;
            }
            let cw = char_adv(c, char_idx_pre);
            char_idx_pre += 1;
            if line_w + cw > wrap_w && byte_pos > line_start_byte {
                if let Some(sp_after) = last_space_after_byte
                    .filter(|&sp| sp > line_start_byte)
                {
                    wrap_starts.push(sp_after);
                    line_start_byte = sp_after;
                    last_space_after_byte = None;
                    // Remaining width = line_w (current) - last_space_w
                    line_w = line_w - last_space_w;
                    last_space_w = 0;
                }
            }
            line_w += cw;
            if c == ' ' {
                last_space_after_byte = Some(byte_pos + 1);
                last_space_w = line_w;
            }
        }
    }

    let mut index = 0usize;
    let mut line_width = 0i32;
    let mut line_y = params.top_spacing as i32;
    let mut visual_line_idx = 0usize;
    let mut next_wrap = wrap_starts.first().copied();
    let mut byte_pos = 0usize;

    for c in text.chars() {
        let c_bytes = c.len_utf8();

        // Visual-line break triggered by word-wrap (byte_pos matches a
        // wrap-start). Behaves like a soft \r/\n: check y range first,
        // then advance line_y and reset line_width.
        while let Some(wp) = next_wrap {
            if wp == byte_pos && byte_pos != 0 {
                if y >= line_y && y < line_y + line_h && x < line_width {
                    return index;
                }
                line_width = 0;
                line_y += line_step;
                visual_line_idx += 1;
                next_wrap = wrap_starts.get(visual_line_idx).copied();
            } else {
                break;
            }
        }

        if c == '\r' || c == '\n' {
            // Match against the full line_step so y values that fall in
            // the 1-px gap between consecutive lines (line_h+1 .. line_step)
            // still resolve to a valid char index. Without this, y exactly
            // at a line boundary (e.g. y=259 with line_step=13) falls
            // through the whole loop and the function returns total-1 =
            // text.length-1, which becomes text.length after 1-based
            // conversion — making `locToCharPos < text.length` FALSE and
            // breaking PageNext's "is there more content below?" probe at
            // scrollTop=0.
            if y >= line_y && y < line_y + line_step && x < line_width {
                return index;
            }
            line_width = 0;
            line_y += line_step;
            // Source \r/\n always advances visual lines; skip the
            // matching wrap_starts entry (already pre-recorded).
            if let Some(wp) = next_wrap {
                if wp == byte_pos + 1 {
                    visual_line_idx += 1;
                    next_wrap = wrap_starts.get(visual_line_idx).copied();
                }
            }
        } else {
            // Match against the full line_step so y values that fall in
            // the 1-px gap between consecutive lines (line_h+1 .. line_step)
            // still resolve to a valid char index. Without this, y exactly
            // at a line boundary (e.g. y=259 with line_step=13) falls
            // through the whole loop and the function returns total-1 =
            // text.length-1, which becomes text.length after 1-based
            // conversion — making `locToCharPos < text.length` FALSE and
            // breaking PageNext's "is there more content below?" probe at
            // scrollTop=0.
            if y >= line_y && y < line_y + line_step && x < line_width {
                return index;
            }
            line_width += char_adv(c, index);
        }
        index += 1;
        byte_pos += c_bytes;
    }
    // Past the last visual line: report the last valid char index
    // (text.chars().count() - 1, clamped to 0) so the caller can read
    // `member.char[y..y+8]` without overflowing past text.length.
    let total = text.chars().count();
    if total == 0 { 0 } else { total - 1 }
}
