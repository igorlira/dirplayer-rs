use std::{
    cell::{Ref, RefCell},
    collections::HashMap,
    rc::Rc,
};

use fxhash::FxHashMap;
use itertools::Itertools;
use log::{debug, log, warn};
use url::Url;

use crate::js_api::ascii_safe;

use crate::{
    director::{enums::ScriptType, file::DirectorFile, lingo::datum::Datum},
    js_api::JsApi,
    player::cast_lib::CastLib,
};
use crate::player::cast_lib::cast_member_ref;
use crate::player::{reserve_player_ref, FontManager};
use crate::player::font::FontRef;

use super::{
    allocator::DatumAllocator,
    bitmap::{bitmap::PaletteRef, manager::{BitmapManager, BitmapRef}, palette_map::PaletteMap},
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
    /// Version counter incremented when palette cache is invalidated.
    /// Used by renderers to know when to clear texture caches.
    pub palette_version: RefCell<u32>,
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
            palette_version: RefCell::new(0),
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
                file_name: if cast_def.is_some() {
                    // Embedded casts: fileName should reference the parent movie (like real Shockwave player).
                    // This ensures Lingo scripts checking fileName.char[end-2..end] = "dcr" work correctly.
                    match &net_manager.base_path {
                        Some(base) => base.join(&dir.file_name).map_or("".to_string(), |u| u.to_string()),
                        None => dir.file_name.to_string(),
                    }
                } else {
                    // External casts: normalize to .cct path
                    normalize_cast_lib_path(&net_manager.base_path, &cast_entry.file_path)
                        .map_or("".to_string(), |it| it.to_string())
                },
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
                preload_mode: cast_entry.preload_settings,
                capital_x: false,
                dir_version: 0,
                palette_id_offset: cast_def.map_or(0, |x| x.palette_id_offset),
            };
            if let Some(cast_def) = cast_def {
                cast.apply_cast_def(dir, cast_def, bitmap_manager, &dir.font_table);
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
        self.resolve_unresolved_palette_refs(bitmap_manager);
        JsApi::dispatch_cast_list_changed();
    }

    /// After all casts (including external) are loaded, resolve bitmap palette refs
    /// where clut_cast_lib was 0 ("not specified"). The bitmap data stores a palette member
    /// number (e.g. 1617) but the wrong cast_lib (defaulted to the bitmap's own castLib).
    /// Search all cast libraries for a member with that number and update the palette_ref
    /// to point to the correct castLib.
    fn resolve_unresolved_palette_refs(&self, bitmap_manager: &mut BitmapManager) {
        // Collect all (bitmap_ref, target_cast_member, current_cast_lib) that need resolution
        let mut to_resolve: Vec<(BitmapRef, i32, i32)> = Vec::new();

        for cast in &self.casts {
            for member in cast.members.values() {
                if let CastMemberType::Bitmap(bm) = &member.member_type {
                    if let Some(bitmap) = bitmap_manager.get_bitmap(bm.image_ref) {
                        if let PaletteRef::Member(ref member_ref) = bitmap.palette_ref {
                            let target_member = member_ref.cast_member;
                            let current_cast_lib = member_ref.cast_lib;
                            // Check if the target member exists as a Palette in the specified castLib.
                            // We must check the member TYPE, not just existence — castlib 2 might
                            // have member #41 as a bitmap, while the actual palette #41 is in castlib 6.
                            let target_cast = self.casts.iter()
                                .find(|c| c.number == current_cast_lib as u32);
                            let is_palette = target_cast
                                .and_then(|c| c.find_member_by_number(target_member as u32))
                                .map_or(false, |m| matches!(m.member_type, CastMemberType::Palette(_)));
                            if !is_palette {
                                debug!(
                                    "palette resolve: bitmap #{} in castLib {} refs palette member {} in castLib {} — not a palette, will search other castLibs",
                                    member.number, cast.number, target_member, current_cast_lib
                                );
                                to_resolve.push((bm.image_ref, target_member, current_cast_lib));
                            }
                        }
                    }
                }
            }
        }

        debug!(
            "palette resolve: {} bitmaps need palette resolution",
            to_resolve.len()
        );

        // Resolve each unresolved palette ref by searching all cast libraries.
        // Search in reverse order (highest castLib first) — Director resolves
        // unqualified member references from the last castLib backwards.
        for (bitmap_ref, target_member, _current_cast_lib) in to_resolve {
            let mut found = false;
            for cast in self.casts.iter().rev() {
                let is_palette = cast.find_member_by_number(target_member as u32)
                    .map_or(false, |m| matches!(m.member_type, CastMemberType::Palette(_)));
                if is_palette {
                    debug!(
                        "palette resolve: found palette member {} in castLib {}",
                        target_member, cast.number
                    );
                    if let Some(bitmap) = bitmap_manager.get_bitmap_mut(bitmap_ref) {
                        bitmap.palette_ref = PaletteRef::Member(cast_member_ref(cast.number as i32, target_member));
                    }
                    found = true;
                    break;
                }
            }
            if !found {
                warn!(
                    "palette resolve: NO palette member {} found in any castLib!",
                    target_member
                );
            }
        }
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
                debug!("Cast {} ({}) - Preload Mode: {}", cast.number, ascii_safe(&cast.file_name), cast.preload_mode);
                // match cast.preload_mode {
                //     0 => {
                //         // Preload: When Needed
                //     }
                //     1 => {
                //         // Preload: After frame one
                //         if reason == CastPreloadReason::AfterFrameOne {
                //             cast.preload(net_manager, bitmap_manager, dir_cache).await;
                //         }
                //     }
                //     2 => {
                //         // Preload: Before frame one
                //         if reason == CastPreloadReason::MovieLoaded {
                //             cast.preload(net_manager, bitmap_manager, dir_cache).await;
                //         }
                //     }
                //     _ => {}
                // }

                // It seems like when the runMode is "Plugin" all casts getting directly download
                // check with mobiles disco in shockwave player
                cast.preload(net_manager, bitmap_manager, dir_cache).await;
            }
        }
    }

    pub fn get_cast(&self, number: u32) -> Result<&CastLib, ScriptError> {
        return self
            .get_cast_or_null(number)
            .ok_or_else(|| ScriptError::new(format!("Cast not found: {}", number)));
    }

    pub fn get_cast_or_null(&self, number: u32) -> Option<&CastLib> {
        return self.casts.get((number as usize).wrapping_sub(1));
    }

    pub fn get_cast_mut(&mut self, number: u32) -> Result<&mut CastLib, ScriptError> {
        return self
            .get_cast_mut_or_null(number)
            .ok_or_else(|| ScriptError::new(format!("Cast not found: {}", number)));
    }

    pub fn get_cast_mut_or_null(&mut self, number: u32) -> Option<&mut CastLib> {
        self.casts.get_mut((number as usize).wrapping_sub(1))
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
                    return Some(cast_member_ref(cast.number as i32, member.number as i32));
                }
            }
        }
        None
    }

    pub fn invalidate_palette_cache(&self) {
        self.palette_cache.replace(None);
        // Increment version counter so renderers know to clear texture caches
        *self.palette_version.borrow_mut() += 1;
    }

    /// Get the current palette version counter
    pub fn palette_version(&self) -> u32 {
        *self.palette_version.borrow()
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

    pub fn find_member_ref_by_name(&self, name: &str) -> Option<CastMemberRef> {
        for cast in &self.casts {
            if let Some(member) = cast.find_member_by_name(name) {
                return Some(cast_member_ref(cast.number as i32, member.number as i32));
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
        let cast_lib = self.find_cast_lib_by_identifier(cast_name_or_num);

        self.find_member_ref_in_cast_by_identifier(member_name_or_num, cast_lib)
    }

    pub fn find_member_ref_in_cast_by_identifier(&self, member_name_or_num: &Datum, cast_lib: Option<&CastLib>) -> Result<Option<CastMemberRef>, ScriptError> {
        let member_ref = match (&member_name_or_num, cast_lib.as_ref()) {
            (Datum::String(name), Some(cast_lib)) => {
                cast_lib.find_member_by_name(name).map(|member| {
                    cast_member_ref(cast_lib.number as i32, member.number as i32)
                })
            }
            (Datum::String(name), None) => self
                .find_member_ref_by_name(name)
                .map(|member_ref| member_ref),
            
            (Datum::Int(num), Some(cast_lib)) => {
                Some(cast_member_ref(cast_lib.number as i32, *num as i32))
            }
            (Datum::Int(num), None) => Some(self
                .find_member_ref_by_number(*num as u32)
                .unwrap_or(cast_member_ref(1, *num))),
            (Datum::Float(num), Some(cast_lib)) => {
                Some(cast_member_ref(cast_lib.number as i32, *num as i32))
            }
            (Datum::Float(num), None) => Some(self
                .find_member_ref_by_number(*num as u32)
                .unwrap_or(cast_member_ref(1, *num as i32))),
            _ => return Err(ScriptError::new(format!(
                "Member number or name type invalid: {} ({})",
                member_name_or_num.type_str(),
                reserve_player_ref(|p| member_name_or_num.repr(p))
            ))),
        };

        if let Some(CastMemberRef { cast_member, .. }) = member_ref && cast_member < 0 {
            return Ok(None);
        }

        Ok(member_ref)
    }

    pub(crate) fn find_cast_lib_by_identifier(&self, cast_name_or_num: Option<&Datum>) -> Option<&CastLib> {
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
        } else if let Some(Datum::CastLib(cast_lib)) = cast_name_or_num {
            self.get_cast_or_null(*cast_lib as u32)
        } else {
            warn!(
                "Cast number or name invalid: {}",
                cast_name_or_num
                    .map(|x| x.type_str())
                    .unwrap_or("None")
            );
            None
        };
        cast_lib
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
        // Direct lookup without slot number conversion for explicit cast_lib references
        if member_ref.cast_lib > 0 {
            let cast = self.get_cast_or_null(member_ref.cast_lib as u32);
            if let Some(cast) = cast {
                return cast.find_member_by_number(member_ref.cast_member as u32);
            }
            return None;
        }
        // Fall back to slot number lookup for global references
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
        self.get_cast_mut_or_null(member_ref.cast_lib as u32)?
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

    /// Search for a script by member number across all cast libraries.
    /// Returns the first script found with the given member number, along with its cast_lib.
    pub fn find_script_in_all_casts(&self, cast_member: i32) -> Option<(i32, &Rc<Script>)> {
        for (idx, cast) in self.casts.iter().enumerate() {
            let cast_lib = (idx + 1) as i32; // Cast libraries are 1-indexed
            if let Some(script) = cast.get_script_for_member(cast_member as u32) {
                return Some((cast_lib, script));
            }
        }
        None
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
                } else if let CastMemberType::Text(text) = &member.member_type {
                    Ok(text.text.to_owned())
                } else {
                    Err(ScriptError::new(format!(
                        "Cast member '{}' is not a field or text member (type: {:?})",
                        member.name, member.member_type.member_type_id()
                    )))
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
        let cast = self.get_cast_mut(member_ref.cast_lib as u32)?;
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

    fn has_pfr_bitmap(font_manager: &FontManager, bitmap_ref: u32) -> Option<FontRef> {
        font_manager
            .fonts
            .iter()
            .find_map(|(font_ref, f)| (f.bitmap_ref == bitmap_ref).then_some(*font_ref))
    }

    /// Load all font cast members into the font manager
    pub fn load_fonts_into_manager(&self, font_manager: &mut FontManager) {
        use web_sys::console;

        let mut loaded_count = 0;
        let mut skipped_count = 0;

        for cast_lib in &self.casts {
            for member in cast_lib.members.values() {
                if let CastMemberType::Font(font_data) = &member.member_type {
                    // Skip font display members (preview text, no bitmap) but NOT PFR fonts
                    if !font_data.preview_text.is_empty() && font_data.bitmap_ref.is_none() {
                        skipped_count += 1;
                        continue;
                    }

                    let font_name = &member.name; // Use member.name, not font_info.name
                    let font_size = font_data.font_info.size;
                    let font_style = font_data.font_info.style;
                    let font_id = font_data.font_info.font_id;
                    let member_number = member.number;

                    debug!(
                        "   📋 Found font member #{}: '{}' (id={}, size={}, style={})",
                        member.number, font_name, font_id, font_size, font_style
                    );

                    // Skip empty font names
                    if font_name.is_empty() {
                        skipped_count += 1;
                        continue;
                    }

                    if let Some(bitmap_ref) = font_data.bitmap_ref {
                        // DEDUPE: if this bitmap font already exists, just ensure id mappings and skip
                        if let Some(existing_ref) = Self::has_pfr_bitmap(font_manager, bitmap_ref as u32) {
                            // Don't overwrite existing mappings; only insert if missing
                            if font_id > 0 {
                                font_manager.font_by_id.entry(font_id).or_insert(existing_ref);
                            }
                            font_manager
                                .font_by_id
                                .entry(member_number as u16)
                                .or_insert(existing_ref);

                            skipped_count += 1;
                            continue;
                        }

                        // This is a PFR font - use its actual dimensions!
                        let char_width = font_data.char_width.unwrap_or(8);
                        let char_height = font_data.char_height.unwrap_or(12);
                        let grid_columns = font_data.grid_columns.unwrap_or(16);
                        let grid_rows = font_data.grid_rows.unwrap_or(8);

                        debug!(
                            "      ✅ PFR font: bitmap_ref={}, dims={}x{}, grid={}x{}",
                            bitmap_ref, char_width, char_height, grid_columns, grid_rows
                        );

                        let font = crate::player::font::BitmapFont {
                            bitmap_ref,
                            char_width,
                            char_height,
                            grid_columns,
                            grid_rows,
                            grid_cell_width: char_width,
                            grid_cell_height: char_height,
                            first_char_num: font_data.first_char_num.unwrap_or(32),
                            char_offset_x: 0,
                            char_offset_y: 0,
                            font_name: font_name.clone(),
                            font_size,
                            font_style,
                            char_widths: font_data.char_widths.clone(),
                            pfr_native_size: font_size,
                        };

                        let rc_font = Rc::new(font);

                        // Collect all name aliases for this font
                        let mut aliases: Vec<String> = Vec::new();
                        aliases.push(font_name.clone());
                        aliases.push(format!("{}_{}_{}", font_name, font_size, font_style));
                        aliases.push(format!("{}_{}_0", font_name, font_size));

                        // Also cache font_info.name if different
                        if !font_data.font_info.name.is_empty()
                            && font_data.font_info.name != *font_name
                        {
                            let pfr_name = &font_data.font_info.name;
                            aliases.push(pfr_name.clone());
                            aliases.push(format!("{}_{}_{}", pfr_name, font_size, font_style));
                            aliases.push(format!("{}_{}_0", pfr_name, font_size));

                            // Add alias with underscores replaced by spaces
                            let spaced = pfr_name.replace('_', " ");
                            if spaced != *pfr_name {
                                aliases.push(spaced);
                            }

                            // Add alias with asterisks replaced by underscores/spaces
                            let unstarred = pfr_name.replace('*', "_");
                            if unstarred != *pfr_name {
                                aliases.push(unstarred.clone());
                                aliases.push(unstarred.replace('_', " "));
                            }

                            // Add prefix-only alias (before first underscore/asterisk/space)
                            // BUT only if the member name doesn't indicate a styled variant
                            // (e.g., don't alias "Verdana Bold *" as just "Verdana")
                            let member_name_upper = font_name.to_ascii_uppercase();
                            let is_styled_variant = member_name_upper.contains("BOLD")
                                || member_name_upper.contains("ITALIC")
                                || member_name_upper.contains("OBLIQUE");
                            if !is_styled_variant {
                                if let Some(prefix_end) = pfr_name.find(|c: char| c == '_' || c == '*' || c == ' ') {
                                    let prefix = &pfr_name[..prefix_end];
                                    if prefix.len() > 1 && prefix != *font_name {
                                        aliases.push(prefix.to_string());
                                    }
                                }
                            }
                        }

                        // Store all aliases in cache (lowercase for case-insensitive lookup)
                        for alias in &aliases {
                            font_manager
                                .font_cache
                                .entry(alias.to_ascii_lowercase())
                                .or_insert_with(|| Rc::clone(&rc_font));
                        }

                        // Store by FontRef
                        let font_ref = font_manager.font_counter;
                        font_manager.font_counter += 1;
                        font_manager.fonts.insert(font_ref, Rc::clone(&rc_font));

                        // Map font_id and member number to this FontRef
                        if font_id > 0 {
                            font_manager.font_by_id.entry(font_id).or_insert(font_ref);
                        }
                        // Also map by member number (STXT formatting runs reference fonts by member number)
                        font_manager.font_by_id.entry(member_number as u16).or_insert(font_ref);

                        console::log_1(&format!(
                            "Loaded PFR font '{}': ref={}, id={}, member={}, char_size={}x{}, first_char={}",
                            font_name, font_ref, font_id, member_number, char_width, char_height,
                            font_data.first_char_num.unwrap_or(32)
                        ).into());

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
                            let full_key = format!("{}_{}_{}", font_name.clone().to_ascii_lowercase(), font_size, font_style);
                            let size_key = format!("{}_{}_0", font_name.clone().to_ascii_lowercase(), font_size);
                            let name_key = font_name.clone().to_ascii_lowercase();

                            // DEDUPE: already cached => don't create a new FontRef
                            if let Some(existing_font) = font_manager.font_cache.get(&full_key) {
                                // Find its FontRef so ids can map to it
                                let existing_ref = font_manager
                                    .fonts
                                    .iter()
                                    .find_map(|(r, f)| Rc::ptr_eq(f, existing_font).then_some(*r));

                                if let Some(existing_ref) = existing_ref {
                                    if font_id > 0 {
                                        font_manager.font_by_id.entry(font_id).or_insert(existing_ref);
                                    }
                                    font_manager
                                        .font_by_id
                                        .entry(member_number as u16)
                                        .or_insert(existing_ref);

                                    skipped_count += 1;
                                    continue;
                                }
                            }

                            // Store in cache
                            font_manager
                                .font_cache
                                .entry(full_key).or_insert_with(|| Rc::clone(&rc_font));
                            font_manager
                                .font_cache
                                .entry(name_key).or_insert_with(|| Rc::clone(&rc_font));
                            font_manager
                                .font_cache
                                .entry(size_key).or_insert_with(|| Rc::clone(&rc_font));

                            // Store by FontRef
                            let font_ref = font_manager.font_counter;
                            font_manager.font_counter += 1;
                            font_manager.fonts.insert(font_ref, rc_font);

                            if font_id > 0 {
                                font_manager.font_by_id.entry(font_id).or_insert(font_ref);
                            }
                            font_manager.font_by_id.entry(member_number as u16).or_insert(font_ref);

                            console::log_1(&format!(
                                "Loaded scaled font '{}': ref={}, member={}, char_size={}x{}",
                                font_name, font_ref, member_number,
                                font_data_clone.char_width, font_data_clone.char_height
                            ).into());

                            loaded_count += 1;
                        } else {
                            warn!(
                                "⚠️  Cannot load font '{}': system font not available",
                                font_name
                            );
                            skipped_count += 1;
                        }
                    }
                }
            }
        }

        if loaded_count > 0 {
            console::log_1(&format!(
                "Font loading complete: {} loaded, {} skipped, {} cache entries, {} id mappings",
                loaded_count, skipped_count,
                font_manager.font_cache.len(),
                font_manager.font_by_id.len()
            ).into());

            // Log all cached font keys to browser console
            let keys: Vec<&String> = font_manager.font_cache.keys().collect();
            debug!("Font cache keys: {:?}", keys);

            // Log font_by_id mappings
            let id_mappings: Vec<(&u16, &crate::player::font::FontRef)> = font_manager.font_by_id.iter().collect();
            debug!("Font by_id mappings: {:?}", id_mappings);
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
