use nohash_hasher::IntMap;
use wasm_bindgen::JsCast;
use wasm_bindgen_futures::JsFuture;

use crate::{
    console_warn, player::{
        bitmap::bitmap::{get_system_default_palette, Bitmap, PaletteRef},
        reserve_player_mut,
    }
};

use super::{
    bitmap::{drawing::CopyPixelsParams, manager::BitmapRef, palette_map::PaletteMap},
    geometry::IntRect,
};
pub type FontRef = u32;

pub struct FontManager {
    pub fonts: IntMap<FontRef, BitmapFont>,
    pub system_font: Option<FontRef>,
    pub font_counter: FontRef,
}

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
}

impl FontManager {
    pub fn new() -> FontManager {
        return FontManager {
            system_font: None,
            fonts: IntMap::default(),
            font_counter: 0,
        };
    }

    pub fn get_system_font(&self) -> Option<&BitmapFont> {
        match self.system_font {
            Some(font_ref) => self.fonts.get(&font_ref),
            None => None,
        }
    }
}

pub async fn player_load_system_font() {
    let window = web_sys::window().unwrap();
    let result = JsFuture::from(window.fetch_with_str("charmap-system.png")).await;

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
                bit_depth: 32, // TODO use a smaller bit depth
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
                    char_offset_y: 1
                };
                let font_ref = player.font_manager.font_counter;
                player.font_manager.font_counter += 1;
                player.font_manager.fonts.insert(font_ref, font);
                player.font_manager.system_font = Some(font_ref);
            });

            console_warn!("Loaded system font image data: {:?}", image_data);
        }
        Err(err) => {
            console_warn!("Error fetching system font: {:?}", err);
            return;
        }
    };
}

pub fn bitmap_font_copy_char(
    font: &BitmapFont,
    font_bitmap: &Bitmap,
    char_num: u8,
    dest: &mut Bitmap,
    dest_x: i16,
    dest_y: i16,
    palettes: &PaletteMap,
    draw_params: &CopyPixelsParams,
) {
    if char_num < font.first_char_num {
        return;
    }
    let char_num = char_num - font.first_char_num;
    let char_x = char_num % font.grid_columns;
    let char_y = char_num / font.grid_columns;

    let src_x = char_x as u16 * font.grid_cell_width + font.char_offset_x;
    let src_y = char_y as u16 * font.grid_cell_height + font.char_offset_y;

    dest.copy_pixels_with_params(
        palettes,
        font_bitmap,
        IntRect::from(
            dest_x as i16,
            dest_y as i16,
            dest_x as i16 + font.char_width as i16,
            dest_y as i16 + font.char_height as i16,
        ),
        IntRect::from(
            src_x as i16,
            src_y as i16,
            src_x as i16 + font.char_width as i16,
            src_y as i16 + font.char_height as i16,
        ),
        &draw_params,
    )
}

pub fn measure_text(text: &str, font: &BitmapFont, line_height: Option<u16>) -> (u16, u16) {
    let mut width = 0;
    let mut line_width = 0;
    let line_height = line_height.unwrap_or(font.char_height);
    let mut height = line_height;
    for c in text.chars() {
        if c == '\r' || c == '\n' {
            height += line_height + 1;
            if line_width > width {
                width = line_width;
            }
            line_width = 0;
        } else {
            line_width += font.char_width + 1;
        }
    }
    if line_width > width {
        width = line_width;
    }
    return (width, height);
}
