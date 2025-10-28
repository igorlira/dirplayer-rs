use std::{
    cell::{Ref, RefCell},
    collections::HashMap,
    rc::Rc,
};

use fxhash::FxHashMap;
use itertools::Itertools;
use log::warn;
use url::Url;

use crate::js_api::ascii_safe;

use crate::{
    director::{enums::ScriptType, file::DirectorFile, lingo::datum::Datum},
    js_api::JsApi,
    player::cast_lib::CastLib,
};

use crate::player::FontManager;

use super::{
    allocator::DatumAllocator,
    bitmap::{manager::BitmapManager, palette_map::PaletteMap},
    cast_lib::{CastLibState, CastMemberRef, INVALID_CAST_MEMBER_REF},
    cast_member::{CastMember, CastMemberType},
    handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
    net_manager::NetManager,
    script::Script,
    ScriptError,
};

pub struct CastManager {
    pub casts: Vec<CastLib>,
    pub movie_script_cache: RefCell<Option<Vec<Rc<Script>>>>,
    pub palette_cache: RefCell<Option<Rc<PaletteMap>>>,
}

const IS_WEB: bool = false;

#[derive(PartialEq)]
pub enum CastPreloadReason {
    MovieLoaded,
    AfterFrameOne,
}

impl CastManager {
    pub const fn empty() -> CastManager {
        CastManager {
            casts: Vec::new(),
            movie_script_cache: RefCell::new(None),
            palette_cache: RefCell::new(None),
        }
    }

    pub async fn load_from_dir(
        &mut self,
        dir: &DirectorFile,
        net_manager: &mut NetManager,
        bitmap_manager: &mut BitmapManager,
        dir_cache: &mut HashMap<Box<str>, DirectorFile>,
    ) {
        let dir_path_uri = &dir.base_path;
        if !IS_WEB || dir_path_uri.host().is_some() {
            net_manager.set_base_path(dir_path_uri.clone());
        }
        let mut casts: Vec<CastLib> = Vec::new();
        for index in 0..dir.cast_entries.len() {
            let cast_entry = &dir.cast_entries[index];
            let cast_def = dir.casts.iter().find(|cast| cast.id == cast_entry.id);
            let mut cast = CastLib {
                name: cast_entry.name.to_owned(),
                file_name: normalize_cast_lib_path(&net_manager.base_path, &cast_entry.file_path)
                    .map_or("".to_string(), |it| it.to_string()),
                number: (index + 1) as u32,
                is_external: cast_def.is_none(),
                state: if cast_def.is_some() {
                    CastLibState::Loaded
                } else {
                    CastLibState::None
                },
                lctx: cast_def.and_then(|x| x.lctx.clone()),
                members: FxHashMap::default(),
                scripts: FxHashMap::default(),
                preload_mode: 0,
                capital_x: false,
                dir_version: 0,
            };
            if let Some(cast_def) = cast_def {
                cast.apply_cast_def(dir, cast_def, bitmap_manager);
                self.clear_movie_script_cache();
            }
            casts.push(cast);
        }
        self.casts = casts;
        self.preload_casts(
            CastPreloadReason::MovieLoaded,
            net_manager,
            bitmap_manager,
            dir_cache,
        )
        .await;
        JsApi::dispatch_cast_list_changed();
    }

    pub async fn preload_casts(
        &mut self,
        reason: CastPreloadReason,
        net_manager: &mut NetManager,
        bitmap_manager: &mut BitmapManager,
        dir_cache: &mut HashMap<Box<str>, DirectorFile>,
    ) {
        for cast in self.casts.iter_mut() {
            if cast.is_external && cast.state == CastLibState::None && !cast.file_name.is_empty() {
                match cast.preload_mode {
                    0 => {
                        // TODO: Load when needed
                        cast.preload(net_manager, bitmap_manager, dir_cache).await;
                    }
                    1 => {
                        if reason == CastPreloadReason::AfterFrameOne {
                            cast.preload(net_manager, bitmap_manager, dir_cache).await;
                        }
                    }
                    2 => {
                        // Before frame one
                        if reason == CastPreloadReason::MovieLoaded {
                            cast.preload(net_manager, bitmap_manager, dir_cache).await;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    pub fn get_cast(&self, number: u32) -> Result<&CastLib, ScriptError> {
        return self
            .get_cast_or_null(number)
            .ok_or_else(|| ScriptError::new(format!("Cast not found: {}", number)));
    }

    pub fn get_cast_or_null(&self, number: u32) -> Option<&CastLib> {
        return self.casts.get(number as usize - 1);
    }

    pub fn get_cast_mut(&mut self, number: u32) -> &mut CastLib {
        return self.casts.get_mut(number as usize - 1).unwrap();
    }

    pub fn get_cast_by_name(&self, name: &str) -> Option<&CastLib> {
        let target = name.to_lowercase();
        self.casts.iter().find(|c| c.name.to_lowercase() == target)
    }

    pub fn find_member_ref_by_number(&self, number: u32) -> Option<CastMemberRef> {
        for cast in &self.casts {
            for member in cast.members.values() {
                if member.number == number
                    || CastMemberRefHandlers::get_cast_slot_number(cast.number, member.number)
                        == number
                {
                    return Some(CastMemberRef {
                        cast_lib: cast.number as i32,
                        cast_member: member.number as i32,
                    });
                }
            }
        }
        None
    }

    pub fn invalidate_palette_cache(&self) {
        self.palette_cache.replace(None);
    }

    pub fn palettes(&self) -> Rc<PaletteMap> {
        let has_cache = self.palette_cache.borrow().is_some();
        if !has_cache {
            let mut result = PaletteMap::new();
            for cast in &self.casts {
                for member in cast.members.values() {
                    if let CastMemberType::Palette(palette) = &member.member_type {
                        let slot_number =
                            CastMemberRefHandlers::get_cast_slot_number(cast.number, member.number);
                        result.insert(slot_number as u32, palette.clone());
                    }
                }
            }
            self.palette_cache.replace(Some(Rc::new(result)));
        }
        self.palette_cache.borrow().as_ref().unwrap().clone()
    }

    pub fn find_member_ref_by_name(&self, name: &String) -> Option<CastMemberRef> {
        for cast in &self.casts {
            if let Some(member) = cast.find_member_by_name(name) {
                return Some(CastMemberRef {
                    cast_lib: cast.number as i32,
                    cast_member: member.number as i32,
                });
            }
        }
        None
    }

    pub fn find_member_ref_by_identifiers(
        &self,
        member_name_or_num: &Datum,
        cast_name_or_num: Option<&Datum>,
        datums: &DatumAllocator,
    ) -> Result<Option<CastMemberRef>, ScriptError> {
        // --- Determine cast library ---
        let cast_lib = if cast_name_or_num.is_none()
            || cast_name_or_num.is_some_and(|x| matches!(x, Datum::Void))
        {
            None
        } else if cast_name_or_num.is_some_and(|x| x.is_string()) {
            if let Ok(cast_name) = cast_name_or_num.unwrap().string_value() {
                self.get_cast_by_name(&cast_name)
            } else {
                warn!(
                    "Invalid cast name: {}",
                    cast_name_or_num.unwrap().type_str()
                );
                None
            }
        } else if cast_name_or_num.is_some_and(|x| x.is_number()) {
            let int_val = cast_name_or_num.unwrap().int_value().unwrap_or(-1);
            if int_val > 0 {
                self.get_cast_or_null(int_val as u32)
            } else {
                None
            }
        } else {
            panic!(
                "Cast number or name invalid: {}",
                cast_name_or_num
                    .map(|x| x.type_str())
                    .unwrap_or("None".to_string())
            )
        };

        let member_ref = match (&member_name_or_num, cast_lib.as_ref()) {
            (Datum::String(name), Some(cast_lib)) => {
                cast_lib.find_member_by_name(name).map(|member| {
                    Ok(Some(CastMemberRef {
                        cast_lib: cast_lib.number as i32,
                        cast_member: member.number as i32,
                    }))
                })
            }
            (Datum::String(name), None) => self
                .find_member_ref_by_name(name)
                .map(|member_ref| Ok(Some(member_ref))),
            (Datum::Int(num), Some(cast_lib)) => {
                cast_lib.find_member_by_number(*num as u32).map(|member| {
                    Ok(Some(CastMemberRef {
                        cast_lib: cast_lib.number as i32,
                        cast_member: member.number as i32,
                    }))
                })
            }
            (Datum::Int(num), None) => self
                .find_member_ref_by_number(*num as u32)
                .map(|member_ref| Ok(Some(member_ref))),
            (Datum::Float(num), Some(cast_lib)) => {
                cast_lib.find_member_by_number(*num as u32).map(|member| {
                    Ok(Some(CastMemberRef {
                        cast_lib: cast_lib.number as i32,
                        cast_member: member.number as i32,
                    }))
                })
            }
            (Datum::Float(num), None) => self
                .find_member_ref_by_number(*num as u32)
                .map(|member_ref| Ok(Some(member_ref))),
            _ => Some(Err(ScriptError::new(format!(
                "Member number or name type invalid: {}",
                member_name_or_num.type_str()
            )))),
        };

        match member_ref {
            None => Ok(None),
            Some(Ok(None)) => Ok(None),
            Some(Ok(Some(member_ref))) => Ok(Some(member_ref)),
            Some(Err(err)) => Err(err),
        }
    }

    pub fn find_member_by_identifiers(
        &self,
        member_name_or_num: &Datum,
        cast_name_or_num: Option<&Datum>,
        datums: &DatumAllocator,
    ) -> Result<Option<&CastMember>, ScriptError> {
        let member_ref =
            self.find_member_ref_by_identifiers(member_name_or_num, cast_name_or_num, datums)?;
        Ok(member_ref.and_then(|member_ref| self.find_member_by_ref(&member_ref)))
    }

    pub fn find_member_by_ref(&self, member_ref: &CastMemberRef) -> Option<&CastMember> {
        let slot_number = CastMemberRefHandlers::get_cast_slot_number(
            member_ref.cast_lib as u32,
            member_ref.cast_member as u32,
        );
        self.find_member_by_slot_number(slot_number)
    }

    pub fn find_member_by_slot_number(&self, slot_number: u32) -> Option<&CastMember> {
        let member_ref = CastMemberRefHandlers::member_ref_from_slot_number(slot_number);
        if member_ref.cast_lib == INVALID_CAST_MEMBER_REF.cast_lib
            || member_ref.cast_member == INVALID_CAST_MEMBER_REF.cast_member
        {
            return None;
        }
        if member_ref.cast_lib > 0 {
            let cast = self.get_cast_or_null(member_ref.cast_lib as u32);
            if let Some(cast) = cast {
                cast.find_member_by_number(member_ref.cast_member as u32)
            } else {
                None
            }
        } else {
            for cast in &self.casts {
                if let Some(member) = cast.find_member_by_number(member_ref.cast_member as u32) {
                    return Some(member);
                }
            }
            return None;
        }
    }

    pub fn find_mut_member_by_ref(
        &mut self,
        member_ref: &CastMemberRef,
    ) -> Option<&mut CastMember> {
        if member_ref.cast_lib <= 0 || member_ref.cast_lib > self.casts.len() as i32 {
            return None;
        }
        self.get_cast_mut(member_ref.cast_lib as u32)
            .find_mut_member_by_number(member_ref.cast_member as u32)
    }

    pub fn get_script_by_ref(&self, member_ref: &CastMemberRef) -> Option<&Rc<Script>> {
        if member_ref.cast_lib == INVALID_CAST_MEMBER_REF.cast_lib
            || member_ref.cast_member == INVALID_CAST_MEMBER_REF.cast_member
        {
            return None;
        } else if let Ok(cast) = self.get_cast(member_ref.cast_lib as u32) {
            cast.get_script_for_member(member_ref.cast_member as u32)
        } else {
            None
        }
    }

    pub fn get_field_value_by_identifiers(
        &self,
        member_name_or_num: &Datum,
        cast_name_or_num: Option<&Datum>,
        datums: &DatumAllocator,
    ) -> Result<String, ScriptError> {
        let member =
            self.find_member_by_identifiers(member_name_or_num, cast_name_or_num, datums)?;
        match member {
            Some(member) => {
                if let CastMemberType::Field(field) = &member.member_type {
                    Ok(field.text.to_owned())
                } else {
                    Err(ScriptError::new(format!("Cast member is not a field")))
                }
            }
            None => Err(ScriptError::new(format!("Cast member not found"))),
        }
    }

    pub fn remove_member_with_ref(
        &mut self,
        member_ref: &CastMemberRef,
    ) -> Result<(), ScriptError> {
        if member_ref.cast_lib <= 0 || member_ref.cast_lib > self.casts.len() as i32 {
            return Err(ScriptError::new(
                "Cannot remove member with invalid cast lib".to_string(),
            ));
        }
        let cast = self.get_cast_mut(member_ref.cast_lib as u32);
        cast.remove_member(member_ref.cast_member as u32);
        Ok(())
    }

    pub fn clear_movie_script_cache(&mut self) {
        let mut cache = self.movie_script_cache.borrow_mut();
        *cache = None;
    }

    pub fn get_movie_scripts(&self) -> Ref<Option<Vec<Rc<Script>>>> {
        if self.movie_script_cache.borrow().is_none() {
            let mut result = Vec::new();
            for cast in &self.casts {
                for script_rc in cast.scripts.values() {
                    if let ScriptType::Movie = script_rc.script_type {
                        result.push(script_rc.clone());
                    }
                }
            }
            self.movie_script_cache.replace(Some(result));
        }
        let cell = self.movie_script_cache.borrow();
        cell
    }

    /// Load all font cast members into the font manager
    pub fn load_fonts_into_manager(&self, font_manager: &mut FontManager) {
        use web_sys::console;

        console::log_1(&"🎨 Starting font loading from cast members...".into());

        let mut loaded_count = 0;
        let mut skipped_count = 0;

        for cast_lib in &self.casts {
            console::log_1(
                &format!(
                    "   Scanning cast lib {} ({} members)",
                    cast_lib.number,
                    cast_lib.members.len()
                )
                .into(),
            );

            for member in cast_lib.members.values() {
                if let CastMemberType::Font(font_data) = &member.member_type {
                    if !font_data.preview_text.is_empty() {
                        console::log_1(
                            &format!(
                  "   ⏭️  Skipping font member #{}: '{}' (has preview text, not a real font)",
                  member.number, member.name
              )
                            .into(),
                        );
                        skipped_count += 1;
                        continue;
                    }

                    let font_name = &member.name; // Use member.name, not font_info.name
                    let font_size = font_data.font_info.size;
                    let font_style = font_data.font_info.style;
                    let font_id = font_data.font_info.font_id;

                    console::log_1(
                        &format!(
                            "   📋 Found font member #{}: '{}' (id={}, size={}, style={})",
                            member.number, font_name, font_id, font_size, font_style
                        )
                        .into(),
                    );

                    // Skip empty font names
                    if font_name.is_empty() {
                        console::log_1(
                            &format!("⊘ Skipping font member {} with empty name", member.number)
                                .into(),
                        );
                        skipped_count += 1;
                        continue;
                    }

                    console::log_1(
                        &format!(
            "📝 Loading font '{}' from cast (id: {}, size: {}, style: {}, member: {})",
            font_name, font_id, font_size, font_style, member.number
          )
                        .into(),
                    );

                    // Check if this is a PFR font with bitmap_ref
                    if let Some(bitmap_ref) = font_data.bitmap_ref {
                        // This is a PFR font - use its actual dimensions!
                        let char_width = font_data.char_width.unwrap_or(8);
                        let char_height = font_data.char_height.unwrap_or(12);
                        let grid_columns = font_data.grid_columns.unwrap_or(16);
                        let grid_rows = font_data.grid_rows.unwrap_or(8);

                        console::log_1(
                            &format!(
                                "      ✅ PFR font: bitmap_ref={}, dims={}x{}, grid={}x{}",
                                bitmap_ref, char_width, char_height, grid_columns, grid_rows
                            )
                            .into(),
                        );

                        let font = crate::player::font::BitmapFont {
                            bitmap_ref,
                            char_width,
                            char_height,
                            grid_columns,
                            grid_rows,
                            grid_cell_width: char_width,
                            grid_cell_height: char_height,
                            first_char_num: 32,
                            char_offset_x: 0,
                            char_offset_y: 0,
                            font_name: font_name.clone(),
                            font_size,
                            font_style,
                        };

                        let rc_font = Rc::new(font);

                        // Create cache keys for different lookup scenarios
                        let full_key = format!("{}_{}_{}", font_name, font_size, font_style);
                        let size_key = format!("{}_{}_0", font_name, font_size);
                        let name_key = font_name.clone();

                        // Store in cache
                        font_manager
                            .font_cache
                            .insert(full_key.clone(), Rc::clone(&rc_font));
                        font_manager
                            .font_cache
                            .insert(name_key.clone(), Rc::clone(&rc_font));
                        font_manager
                            .font_cache
                            .insert(size_key.clone(), Rc::clone(&rc_font));

                        // Also cache font_info.name if different
                        if !font_data.font_info.name.is_empty()
                            && font_data.font_info.name != *font_name
                        {
                            font_manager
                                .font_cache
                                .insert(font_data.font_info.name.clone(), Rc::clone(&rc_font));
                        }

                        // Store by FontRef
                        let font_ref = font_manager.font_counter;
                        font_manager.font_counter += 1;
                        font_manager.fonts.insert(font_ref, rc_font);

                        // Map the font_id to this FontRef
                        if font_id > 0 {
                            font_manager.font_by_id.insert(font_id, font_ref);
                        }

                        console::log_1(
                            &format!(
                                "      ✅ Loaded PFR: ref={}, char_size={}x{}",
                                font_ref, char_width, char_height
                            )
                            .into(),
                        );

                        loaded_count += 1;
                    } else {
                        // Not a PFR font - use system font template with scaling
                        if let Some(system_font) = font_manager.get_system_font() {
                            let mut font_data_clone = (*system_font).clone();

                            font_data_clone.font_name = font_name.clone();
                            font_data_clone.font_size = font_size;
                            font_data_clone.font_style = font_style;

                            let scale_factor = if font_size > 0 {
                                font_size as f32 / 12.0
                            } else {
                                1.0
                            };

                            font_data_clone.char_width =
                                (system_font.char_width as f32 * scale_factor)
                                    .max(1.0)
                                    .ceil() as u16;
                            font_data_clone.char_height =
                                (system_font.char_height as f32 * scale_factor)
                                    .max(1.0)
                                    .ceil() as u16;
                            font_data_clone.grid_cell_width =
                                (system_font.grid_cell_width as f32 * scale_factor)
                                    .max(1.0)
                                    .ceil() as u16;
                            font_data_clone.grid_cell_height =
                                (system_font.grid_cell_height as f32 * scale_factor)
                                    .max(1.0)
                                    .ceil() as u16;

                            let rc_font = Rc::new(font_data_clone.clone());

                            // Create cache keys
                            let full_key = format!("{}_{}_{}", font_name, font_size, font_style);
                            let size_key = format!("{}_{}_0", font_name, font_size);
                            let name_key = font_name.clone();

                            // Store in cache
                            font_manager
                                .font_cache
                                .insert(full_key, Rc::clone(&rc_font));
                            font_manager
                                .font_cache
                                .insert(name_key, Rc::clone(&rc_font));
                            font_manager
                                .font_cache
                                .insert(size_key, Rc::clone(&rc_font));

                            // Store by FontRef
                            let font_ref = font_manager.font_counter;
                            font_manager.font_counter += 1;
                            font_manager.fonts.insert(font_ref, rc_font);

                            if font_id > 0 {
                                font_manager.font_by_id.insert(font_id, font_ref);
                            }

                            console::log_1(
                                &format!(
                "      ✅ Loaded (scaled): ref={}, scale={:.2}x, char_size={}x{}",
                font_ref, scale_factor, font_data_clone.char_width, font_data_clone.char_height
              )
                                .into(),
                            );

                            loaded_count += 1;
                        } else {
                            console::log_1(
                                &format!(
                                    "⚠️  Cannot load font '{}': system font not available",
                                    font_name
                                )
                                .into(),
                            );
                            skipped_count += 1;
                        }
                    }
                }
            }
        }

        console::log_1(
            &format!(
                "🎨 Font loading complete: {} loaded, {} skipped",
                loaded_count, skipped_count
            )
            .into(),
        );
        console::log_1(
            &format!(
                "   Cache: {} entries, {} font_id mappings",
                font_manager.font_cache.len(),
                font_manager.font_by_id.len()
            )
            .into(),
        );

        // Log all cached fonts for debugging
        console::log_1(&"   Cached fonts:".into());
        for (key, font) in &font_manager.font_cache {
            console::log_1(
                &format!(
                    "      '{}' -> {} ({}pt, style={}, {}x{})",
                    key,
                    font.font_name,
                    font.font_size,
                    font.font_style,
                    font.char_width,
                    font.char_height
                )
                .into(),
            );
        }
    }
}

fn normalize_cast_lib_path(base_path: &Option<Url>, file_path: &str) -> Option<String> {
    if file_path.is_empty() {
        return None;
    }

    // bind temporary String to a variable so slices can borrow from it
    let normalized = file_path.replace("\\", "/");

    // split on both slashes and colons
    let parts: Vec<&str> = normalized.split(&['/', ':'][..]).collect();
    let file_base_name = parts.last().unwrap_or(&"");

    let cast_file_name = if file_base_name.contains('.') {
        let dot_parts: Vec<&str> = file_base_name.split('.').collect();
        format!("{}.cct", dot_parts[..dot_parts.len() - 1].join("."))
    } else {
        format!("{}.cct", file_base_name)
    };

    let result = match base_path {
        Some(base_path) => base_path.join(&cast_file_name).unwrap().to_string(),
        None => cast_file_name,
    };

    Some(ascii_safe(&result))
}
