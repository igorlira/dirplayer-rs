use fxhash::FxHashMap;
use log::warn;
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
pub type FontRef = u32;

pub struct FontManager {
    pub fonts: FxHashMap<FontRef, Rc<BitmapFont>>,
    pub system_font: Option<Rc<BitmapFont>>,
    pub font_counter: FontRef,
    pub font_cache: HashMap<String, Rc<BitmapFont>>, // Cache for loaded fonts by name
    pub font_by_id: HashMap<u16, FontRef>,           // Map font_id to FontRef
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
}

pub struct DrawTextParams<'a> {
    pub font: &'a BitmapFont,
    pub line_height: Option<u16>,
    pub line_spacing: u16,
    pub top_spacing: i16,
}

impl FontManager {
    pub fn new() -> FontManager {
        return FontManager {
            fonts: FxHashMap::default(),
            system_font: None,
            font_counter: 0,
            font_cache: HashMap::new(),
            font_by_id: HashMap::new(),
        };
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
        // Check cache first
        if self.font_cache.contains_key(font_name) {
            return self.font_cache.get(font_name).map(|v| &**v);
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
        self.font_cache.get(font_name).map(|v| &**v)
    }

    /// Load a font from cast members by searching the cast manager
    pub fn load_font_from_cast(
        &mut self,
        font_name: &str,
        cast_manager: &CastManager,
        size: Option<u16>,
        style: Option<u8>,
    ) -> Option<Rc<BitmapFont>> {
        let cache_key = format!("{}_{}_{}", font_name, size.unwrap_or(0), style.unwrap_or(0));

        for cast_lib in &cast_manager.casts {
            for member in cast_lib.members.values() {
                if let CastMemberType::Font(font_data) = &member.member_type {
                    // Check BOTH the font_info.name AND the member.name
                    let name_matches =
                        font_data.font_info.name == font_name || member.name == font_name;
                    let size_matches = size.is_none() || size == Some(font_data.font_info.size);
                    let style_matches = style.is_none() || style == Some(font_data.font_info.style);

                    if name_matches && size_matches && style_matches {
                        web_sys::console::log_1(
                            &format!(
                                "✅ Found matching font: member.name='{}', font_info.name='{}'",
                                member.name, font_data.font_info.name
                            )
                            .into(),
                        );

                        // Check if this font has a bitmap_ref from PFR parsing
                        if let Some(bitmap_ref) = font_data.bitmap_ref {
                            web_sys::console::log_1(
                                &format!("✅ Found PFR font with bitmap_ref: {}", bitmap_ref)
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
                                first_char_num: 32,
                                char_offset_x: 0, // IMPORTANT: No offset for PFR fonts
                                char_offset_y: 0, // IMPORTANT: No offset for PFR fonts
                                font_name: member.name.clone(),
                                font_size: font_data.font_info.size,
                                font_style: font_data.font_info.style,
                            };

                            let rc_font = Rc::new(font);

                            // Cache under ALL name variations
                            web_sys::console::log_1(
                                &format!(
                                    "📦 Caching font as: '{}', '{}', '{}'",
                                    cache_key, font_name, member.name
                                )
                                .into(),
                            );

                            self.font_cache
                                .insert(cache_key.clone(), Rc::clone(&rc_font));
                            self.font_cache
                                .insert(font_name.to_string(), Rc::clone(&rc_font));
                            self.font_cache
                                .insert(member.name.clone(), Rc::clone(&rc_font));

                            if font_data.font_info.name != member.name
                                && font_data.font_info.name != font_name
                            {
                                self.font_cache
                                    .insert(font_data.font_info.name.clone(), Rc::clone(&rc_font));
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

                            let scale_factor = font_data.font_info.size as f32 / 12.0;
                            new_font.char_width =
                                (new_font.char_width as f32 * scale_factor) as u16;
                            new_font.char_height =
                                (new_font.char_height as f32 * scale_factor) as u16;

                            let rc_font = Rc::new(new_font);
                            self.font_cache
                                .insert(cache_key.clone(), Rc::clone(&rc_font));
                            if !self.font_cache.contains_key(font_name) {
                                self.font_cache
                                    .insert(font_name.to_string(), Rc::clone(&rc_font));
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
        let cache_key = format!("{}_{}_{}", font_name, size.unwrap_or(0), style.unwrap_or(0));

        if let Some(font) = self.font_cache.get(&cache_key) {
            return Some(Rc::clone(font));
        }

        if let Some(cast_mgr) = cast_manager {
            if let Some(font) = self.load_font_from_cast(font_name, cast_mgr, size, style) {
                return Some(font);
            }
        }

        None
    }

    pub fn get_best_font(&mut self, font_name: &str) -> Option<&BitmapFont> {
        // Just use get directly - no lifetime conflicts
        self.font_cache.get(font_name).map(|v| &**v)
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
                    .insert("System".to_string(), font.into());

                web_sys::console::log_1(&"✅ System font loaded successfully".into());
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

pub fn measure_text(
    text: &str,
    font: &BitmapFont,
    line_height: Option<u16>,
    line_spacing: u16,
    top_spacing: i16,
) -> (u16, u16) {
    let mut width = 0;
    let mut line_width = 0;
    let line_height = line_height.unwrap_or(font.char_height);
    let mut height = (top_spacing + line_height as i16) as u16;
    let mut index = 0;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if line_width > width {
                width = line_width;
            }
            line_width = 0;
        } else {
            if line_width == 0 && index > 0 {
                height += (line_height as i16 + line_spacing as i16 + 1) as u16;
            }
            line_width += font.char_width + 1;
        }
        index += 1;
    }
    if line_width > width {
        width = line_width;
    }
    return (width, height);
}

pub fn _get_text_char_pos(text: &str, params: &DrawTextParams, char_index: usize) -> (i16, i16) {
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
            y += params.line_height.unwrap_or(params.font.char_height) as i16
                + params.line_spacing as i16
                + 1;
        } else {
            if line_index == char_index {
                return (x, y);
            }
            line_width += params.font.char_width as i16 + 1;
        }
        line_index += 1;
    }
    if line_width > x {
        x = line_width;
    }
    return (x, y);
}

pub fn get_text_index_at_pos(text: &str, params: &DrawTextParams, x: i32, y: i32) -> usize {
    let mut index = 0;
    let mut line_width = 0;
    let mut line_y = params.top_spacing as i32;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            if y >= line_y
                && y < line_y + params.line_height.unwrap_or(params.font.char_height) as i32
            {
                if x < line_width {
                    return index;
                }
            }
            if line_width > x {
                line_width = 0;
            }
            line_y += params.line_height.unwrap_or(params.font.char_height) as i32
                + params.line_spacing as i32
                + 1;
        } else {
            if y >= line_y
                && y < line_y + params.line_height.unwrap_or(params.font.char_height) as i32
            {
                if x < line_width {
                    return index;
                }
            }
            line_width += params.font.char_width as i32 + 1;
        }
        index += 1;
    }
    return index;
}
