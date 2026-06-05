use std::{cell::RefCell, collections::HashMap, rc::Rc};

use fxhash::FxHashMap;
use url::Url;

use crate::{
    director::{
        cast::CastDef, chunks::sound::SoundChunk, enums::{ScriptType, BitmapInfo, SoundInfo},
        file::{read_director_file_bytes, DirectorFile},
        lingo::{datum::Datum, script::ScriptContext},
    }, js_api::JsApi, player::{cast_member::ScriptMember, ci_string::CiString, symbols::{builtin::BuiltInSymbol, symbol::Symbol}}, utils::{get_base_url, get_basename_no_extension, log_i}
};

use super::{
    allocator::DatumAllocator,
    bitmap::{
        bitmap::{Bitmap, BuiltInPalette, PaletteRef},
        manager::BitmapManager,
    },
    cast_member::{
        BitmapMember, CastMember, CastMemberType, FieldMember, PaletteMember, SoundMember,
        TextMember,
    },
    datum_ref::DatumRef,
    handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
    net_manager::NetManager,
    net_task::NetResult,
    reserve_player_mut,
    script::Script,
    ScriptError, PLAYER_OPT,
};

pub type CastLibNumber = u32;
pub type CastMemberNumber = u32;
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CastLibState {
    None,
    Loading,
    Loaded,
}

pub struct CastLib {
    pub name: String,
    pub file_name: String,
    pub number: u32,
    pub is_external: bool,
    pub state: CastLibState,
    pub lctx: Option<ScriptContext>,
    pub members: FxHashMap<u32, CastMember>,
    pub scripts: FxHashMap<u32, Rc<Script>>,
    pub name_symbols: Vec<Symbol>,
    pub preload_mode: u16,
    pub capital_x: bool,
    pub dir_version: u16,
    /// Offset to adjust bitmap clutId from Config-based to MCsL-based member numbering.
    pub palette_id_offset: i16,
    /// Lazy lowercased-name → lowest member number index for `find_member_by_name`.
    /// Built on first lookup and reused until a member is inserted/removed/renamed
    /// (see `invalidate_name_index`). Replaces an O(members) linear scan per call —
    /// the Habbo preloader's `FindCastNumber` hammers name lookups in tight loops.
    pub name_index: RefCell<Option<FxHashMap<String, u32>>>,
    /// Director Fmap/VWFM font-table snapshot: font_id → font name (e.g.
    /// "Arial", "Arial Bold", "Arial Italic"). Kept on the cast lib so
    /// per-run `font_id`s from STXT can be resolved to actual names at
    /// runtime — `.font` and `.fontStyle` chunk getters need this to
    /// distinguish bold variants from italic variants. Bit-flag heuristics
    /// (e.g. `font_id & 0x8000 = bold`) don't generalise — different movies
    /// pack the table differently and the only authoritative mapping is
    /// the file's own font table.
    pub font_table: HashMap<u16, String>,
}

impl CastLib {
    pub fn max_member_id(&self) -> u32 {
        *self.members.keys().max().unwrap_or(&0)
    }

    pub fn first_free_member_id(&self) -> u32 {
        let max_member = 5000; // TODO where from?
        for i in 1..max_member {
            if !self.members.contains_key(&i) {
                return i;
            }
        }
        0
    }

    pub fn remove_member(&mut self, number: u32) {
        // TODO remove from movie script cache
        self.members.remove(&number);
        self.scripts.remove(&number);
        self.invalidate_name_index();
        JsApi::on_cast_member_name_changed(CastMemberRefHandlers::get_cast_slot_number(
            self.number,
            number,
        ));
        JsApi::dispatch_cast_member_list_changed(self.number);
    }

    pub async fn preload(
        &mut self,
        net_manager: &mut NetManager,
        bitmap_manager: &mut BitmapManager,
        dir_cache: &mut HashMap<Box<str>, DirectorFile>,
    ) {
        let file_name = self.file_name.clone();
        if file_name.is_empty() {
            return;
        } else if let Some(cached_file) = dir_cache.get(&*file_name) {
            let __t = crate::player::bench_now_ms();
            self.load_from_dir_file(cached_file, &file_name, bitmap_manager);
            crate::player::cast_diag_add_apply(crate::player::bench_now_ms() - __t);
        } else {
            log::debug!("Loading cast {} into castLib {} ('{}')", self.file_name, self.number, self.name);
            self.state = CastLibState::Loading;
            let task_id = net_manager.preload_net_thing(self.file_name.clone());
            if !net_manager.is_task_done(Some(task_id)) {
                let __t = crate::player::bench_now_ms();
                net_manager.await_task(task_id).await;
                crate::player::cast_diag_add_net(crate::player::bench_now_ms() - __t);
            }
            let task = net_manager.get_task(task_id).unwrap();
            let result = net_manager.get_task_result(Some(task_id)).unwrap();
            self.on_cast_preload_result(&result, &task.resolved_url, bitmap_manager, dir_cache);
        }
    }

    fn on_cast_preload_result(
        &mut self,
        result: &NetResult,
        resolved_url: &Url,
        bitmap_manager: &mut BitmapManager,
        dir_cache: &mut HashMap<Box<str>, DirectorFile>,
    ) {
        let load_file_name = resolved_url.as_str();
        if let Ok(cast_bytes) = result {
            let __tp = crate::player::bench_now_ms();
            let cast_file = read_director_file_bytes(
                cast_bytes,
                &resolved_url.to_string(),
                &get_base_url(resolved_url).to_string(),
            );
            crate::player::cast_diag_add_parse(crate::player::bench_now_ms() - __tp);
            if let Ok(cast_file) = cast_file {
                dir_cache.insert(load_file_name.into(), cast_file);
                let cast_file = dir_cache.get(load_file_name).unwrap();
                let __ta = crate::player::bench_now_ms();
                self.load_from_dir_file(&cast_file, load_file_name, bitmap_manager);
                crate::player::cast_diag_add_apply(crate::player::bench_now_ms() - __ta);
                // We return here because the function `load_from_dir_file()`
                // has changed our `state` to `Loaded` and we want to keep this.
                return;
            } else {
                log::debug!("Could not parse {load_file_name}");
            }
        } else {
            // ~650 null.cst pool slots fail to fetch during resetCastLibs;
            // keep this off the console (log::debug) to avoid flooding DevTools.
            log::debug!("Fetching {load_file_name} failed");
        }
        self.state = CastLibState::None;
    }

    pub fn find_member_by_number(&self, number: u32) -> Option<&CastMember> {
        self.members.get(&number)
    }

    pub fn find_mut_member_by_number(&mut self, number: u32) -> Option<&mut CastMember> {
        self.members.get_mut(&number)
    }

    /// Drop the lazily-built name index. Called whenever the member set or a
    /// member name changes, so the next `find_member_by_name` rebuilds it.
    pub fn invalidate_name_index(&self) {
        self.name_index.replace(None);
    }

    pub fn find_member_by_name(&self, name: &str) -> Option<&CastMember> {
        // Director returns the lowest-numbered member when duplicates exist in the
        // same cast. Build (once) a lowercased-name → lowest-number index so repeated
        // lookups are O(1) instead of an O(members) scan per call.
        if self.name_index.borrow().is_none() {
            let mut index: FxHashMap<String, u32> = FxHashMap::default();
            for member in self.members.values() {
                let key = member.name.to_ascii_lowercase();
                index
                    .entry(key)
                    .and_modify(|n| {
                        if member.number < *n {
                            *n = member.number;
                        }
                    })
                    .or_insert(member.number);
            }
            self.name_index.replace(Some(index));
        }

        let number = {
            let idx_ref = self.name_index.borrow();
            let idx = idx_ref.as_ref().unwrap();
            // Avoid allocating a lowercased key on every lookup: most callers
            // (Habbo's normalizeCastName output) already pass a lowercase name,
            // so probe the index directly with `&str` and only allocate when the
            // name actually contains uppercase ASCII.
            if name.bytes().any(|b| b.is_ascii_uppercase()) {
                let lookup = name.to_ascii_lowercase();
                idx.get(lookup.as_str()).copied()
            } else {
                idx.get(name).copied()
            }
        };
        number.and_then(|n| self.members.get(&n))
    }


    fn clear(&mut self) {
        // Clear regardless of state. The previous early-return-when-not-Loaded
        // guard left stale members in place when a swap-in-place reload went
        // through the network path of `preload`: that path sets
        // `state = Loading` *before* awaiting the fetch, and by the time
        // `load_from_dir_file` calls `clear()` the guard short-circuited so
        // the OLD cast's members survived. `apply_cast_def` then merged the
        // new cast on top, leaving any slot the new cast didn't redefine
        // pinned to the previous cast's content (e.g. Coke Studios' first-time
        // swap from one public studio to another bled walls/floor/decor from
        // the previously visited studio).
        if self.members.is_empty()
            && self.scripts.is_empty()
            && self.lctx.is_none()
            && self.state == CastLibState::None
        {
            return;
        }
        self.members.clear();
        self.scripts.clear();
        self.lctx = None;
        self.state = CastLibState::None;
        self.invalidate_name_index();

        JsApi::dispatch_cast_member_list_changed(self.number);
    }

    fn set_name(&mut self, name: String) {
        if name != self.name {
            self.name = name;
            JsApi::dispatch_cast_name_changed(self.number);
        }
    }

    pub fn set_prop(
        &mut self,
        prop: Symbol,
        value: Datum,
        datums: &DatumAllocator,
    ) -> Result<(), ScriptError> {
        // TODO
        match prop.into_builtin_or_error()? {
            BuiltInSymbol::PreloadMode => {
                self.preload_mode = value.int_value()? as u16;
            }
            BuiltInSymbol::Name => {
                self.set_name(value.string_value()?);
            }
            BuiltInSymbol::FileName => {
                self.file_name = value.string_value()?;
            }
            _ => {
                return Err(ScriptError::new(format!(
                    "Cannot set castLib property {}",
                    prop
                )));
            }
        };
        Ok(())
    }

    pub fn get_prop(&self, prop: Symbol) -> Result<Datum, ScriptError> {
        match prop.into_builtin_or_error()? {
            BuiltInSymbol::PreloadMode => Ok(Datum::Int(self.preload_mode as i32)),
            BuiltInSymbol::FileName => {
                // Only return the fileName if the cast is actually loaded.
                // External casts with preloadMode=0 ("When Needed") have file_name set
                // in the movie structure but aren't loaded yet. Scripts like PreloadCast
                // compare castLib.fileName to decide whether to download — returning the
                // configured name for an unloaded cast would skip the download.
                if self.is_external && self.state == CastLibState::None {
                    Ok(Datum::String(String::new()))
                } else {
                    Ok(Datum::String(self.file_name.clone()))
                }
            }
            BuiltInSymbol::Number => Ok(Datum::Int(self.number as i32)),
            BuiltInSymbol::Name => Ok(Datum::String(self.name.clone())),
            BuiltInSymbol::NumberOfCastMembers | BuiltInSymbol::NumberOfMembers => {
                // Director semantics: the highest member slot number in use,
                // not the population count. Casts routinely have gaps, and
                // Lingo code like `repeat with i = 1 to the number of
                // castMembers of castLib "X"` relies on this to reach every
                // populated slot (including dynamically created members at
                // high numbers) — otherwise cleanup loops silently skip them.
                Ok(Datum::Int(self.max_member_id() as i32))
            }
            _ => Err(ScriptError::new(format!(
                "Cannot get castLib property {}",
                prop
            ))),
        }
    }

    fn load_from_dir_file(
        &mut self,
        file: &DirectorFile,
        load_file_name: &str,
        bitmap_manager: &mut BitmapManager,
    ) {
        self.clear();
        // TODO file.parseScripts

        self.file_name = load_file_name.to_owned();
        self.state = CastLibState::Loaded;
        if self.name.is_empty() {
            self.set_name(get_basename_no_extension(load_file_name));
        }
        if let Some(cast_def) = file.casts.first() {
            log::debug!(
                "Applying cast def to castLib {} ('{}'): {} members",
                self.number,
                self.name,
                cast_def.members.len()
            );
            self.apply_cast_def(file, cast_def, bitmap_manager, &file.font_table);
        } else {
            log_i(
                format_args!(
                    "No cast def found in file {} for castLib {} ('{}')",
                    load_file_name,
                    self.number,
                    self.name
                )
                .to_string()
                .as_str(),
            );
        }
    }

    pub fn apply_cast_def(
        &mut self,
        _: &DirectorFile,
        cast_def: &CastDef,
        bitmap_manager: &mut BitmapManager,
        font_table: &HashMap<u16, String>,
    ) {
        self.lctx = cast_def.lctx.clone();
        self.name_symbols = self.lctx.as_ref()
            .map(|lctx| lctx.names.iter().map(|n| Symbol::from_str(n)).collect())
            .unwrap_or_default();
        self.capital_x = cast_def.capital_x;
        self.dir_version = cast_def.dir_version;
        self.palette_id_offset = cast_def.palette_id_offset;
        self.font_table = font_table.clone();
        self.state = CastLibState::Loaded;
        for (id, member_def) in &cast_def.members {
            self.insert_member(
                *id,
                CastMember::from(self.number, *id, member_def, &self.lctx, bitmap_manager, self.dir_version, self.palette_id_offset, font_table),
            );
            JsApi::on_cast_member_name_changed(CastMemberRefHandlers::get_cast_slot_number(
                self.number,
                *id,
            ));
        }
        JsApi::dispatch_cast_member_list_changed(self.number);
        unsafe {
            let player_mut = &mut PLAYER_OPT.as_mut().unwrap();

            player_mut.movie.cast_manager.clear_movie_script_cache();
            player_mut.movie.cast_manager.invalidate_member_name_cache();
            player_mut.movie.cast_manager.load_fonts_into_manager(&mut player_mut.font_manager);
        };
    }

    pub fn insert_member(&mut self, number: u32, member: CastMember) {
        if let CastMemberType::Script(script_member) = &member.member_type {
            let script_def = self
                .lctx
                .as_ref()
                .and_then(|lctx| lctx.scripts.get(&script_member.script_id));

            if let Some(script_def) = script_def {
                let mut handler_names = Vec::new();
                let mut handler_names_raw = Vec::new();
                let mut handler_name_map = FxHashMap::default();
                for handler in &script_def.handlers {
                    let handler_name = &self.lctx.as_ref().unwrap().names[handler.name_id as usize];
                    let handler_name_symbol = Symbol::from_str(handler_name);
                    handler_name_map.insert(handler_name_symbol, Rc::new(handler.clone()));
                    handler_names.push(handler_name_symbol);
                    handler_names_raw.push(handler_name.clone());
                }

                let property_names = script_def
                    .property_name_ids
                    .iter()
                    .map(|id| Symbol::from_str(&self.lctx.as_ref().unwrap().names[*id as usize]));
                let mut properties = FxHashMap::default();
                for name in property_names {
                    properties.insert(name, DatumRef::Void);
                }

                let script = Script {
                    member_ref: cast_member_ref(self.number as i32, number as i32),
                    name: (&member.name).to_owned(),
                    chunk: script_def.clone(),
                    script_type: script_member.script_type,
                    handlers: handler_name_map,
                    handler_names,
                    handler_names_raw,
                    properties: RefCell::new(properties),
                };
                // JS Lingo diagnostic: decode and disassemble each XDR-wrapped JSScript
                // in the literal data area. Phase 1 — read-only; translator hook lands later.
                crate::player::js_lingo_loader::diagnose_js_script(&script);
                self.scripts.insert(number, Rc::new(script));
            }
        } else if let CastMemberType::Palette(_) = &member.member_type {
            reserve_player_mut(|player| {
                player.movie.cast_manager.invalidate_palette_cache();
            });
        }

        self.members.insert(number, member);
        self.invalidate_name_index();
    }

    pub fn create_member_at(
        &mut self,
        number: u32,
        member_type: &str,
        bitmap_manager: &mut BitmapManager,
    ) -> Result<CastMemberRef, ScriptError> {
        let member = match member_type {
            "field" => Ok(CastMember::new(
                number,
                CastMemberType::Field(FieldMember::new()),
            )),
            "text" => Ok(CastMember::new(
                number,
                CastMemberType::Text(TextMember::new()),
            )),
            "bitmap" => {
                let bitmap = Bitmap::new(
                    0,
                    0,
                    32,
                    32,
                    0,
                    PaletteRef::BuiltIn(BuiltInPalette::GrayScale),
                );
                let bitmap_ref = bitmap_manager.add_bitmap(bitmap);
                Ok(CastMember::new(
                    number,
                    CastMemberType::Bitmap(BitmapMember {
                        image_ref: bitmap_ref,
                        reg_point: (0, 0),
                        script_id: 0,
                        member_script_ref: None,
                        info: BitmapInfo::default(),
                    }),
                ))
            }
            "palette" => Ok(CastMember::new(
                number,
                CastMemberType::Palette(PaletteMember::new()),
            )),
            // `new(#sound, castLib)` creates an empty sound member; its media is
            // populated later, typically via `importFileInto` (Director 11.5
            // Scripting Dictionary, `new()` / `importFileInto()`). Habbo's
            // Download Manager relies on this for streamed trax samples.
            "sound" => Ok(CastMember::new(
                number,
                CastMemberType::Sound(SoundMember {
                    info: SoundInfo::default(),
                    sound: SoundChunk::default(),
                    cue_point_times: Vec::new(),
                    cue_point_names: Vec::new(),
                }),
            )),
            "script" => Ok(CastMember::new(
                number,
                CastMemberType::Script(ScriptMember {
                    script_id: 0,
                    script_type: ScriptType::Movie,
                    name: String::new(),
                }),
            )),
            _ => Err(ScriptError::new(format!(
                "Cannot create member of type {}",
                member_type
            ))),
        }?;
        self.insert_member(number, member);
        JsApi::dispatch_cast_member_list_changed(self.number);
        Ok(cast_member_ref(self.number as i32, number as i32))
    }

    pub fn get_script_for_member(&self, number: u32) -> Option<&Rc<Script>> {
        // Direct path: `number` is a cast-member slot that holds a Script
        // member — this is how scripts authored as standalone behaviors are
        // registered (see `insert_member`, which inserts `scripts[number]`).
        if let Some(script) = self.scripts.get(&number) {
            return Some(script);
        }

        // Fallback: `number` is an lctx-script-id (the value stored in
        // `member_info.header.script_id` for non-Script members like Field
        // and Text). In D11+ movies this id may NOT equal the cast-member
        // slot — e.g. a field with `header.script_id=10` may have its
        // actual script cast member elsewhere. Walk the cast looking for
        // a Script-type member whose own `script_id` matches, then return
        // its registered script.
        for (slot, member) in &self.members {
            if let CastMemberType::Script(script_member) = &member.member_type {
                if script_member.script_id == number {
                    return self.scripts.get(slot);
                }
            }
        }
        None
    }

    pub fn get_behavior_script_from_lctx(&mut self, script_id: u32) -> Option<Rc<Script>> {
        // Use an offset to avoid collision with cast member numbers
        // Behavior scripts are stored at script_id + 1000000
        let cache_key = script_id + 1000000;
        
        // Check if already cached
        if let Some(cached) = self.scripts.get(&cache_key) {
            return Some(cached.clone());
        }
        
        // Get script chunk from lctx.scripts
        let script_chunk = self.lctx.as_ref()?.scripts.get(&script_id)?;
        
        // Build handler map
        let mut handler_names = Vec::new();
        let mut handler_names_raw = Vec::new();
        let mut handler_name_map = FxHashMap::default();
        for handler in &script_chunk.handlers {
            let handler_name = &self.lctx.as_ref().unwrap().names[handler.name_id as usize];
            let handler_name_symbol = Symbol::from_str(handler_name);
            handler_name_map.insert(handler_name_symbol, Rc::new(handler.clone()));
            handler_names.push(handler_name_symbol);
            handler_names_raw.push(handler_name.clone());
        }

        // Build properties
        let property_names = script_chunk
            .property_name_ids
            .iter()
            .map(|id| Symbol::from_str(&self.lctx.as_ref().unwrap().names[*id as usize]));
        let mut properties = FxHashMap::default();
        for name in property_names {
            properties.insert(name, DatumRef::Void);
        }

        let script = Rc::new(Script {
            member_ref: cast_member_ref(self.number as i32, cache_key as i32),
            name: format!("BehaviorScript_{}", script_id),
            chunk: script_chunk.clone(),
            script_type: ScriptType::Member,
            handlers: handler_name_map,
            handler_names,
            handler_names_raw,
            properties: RefCell::new(properties),
        });
        
        // Cache it with the offset key
        self.scripts.insert(cache_key, script.clone());
        
        Some(script)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Copy)]
pub struct CastMemberRef {
    pub cast_lib: i32,
    pub cast_member: i32,
}

pub const INVALID_CAST_MEMBER_REF: CastMemberRef = CastMemberRef {
    cast_lib: -1,
    cast_member: -1,
};
pub const NULL_CAST_MEMBER_REF: CastMemberRef = CastMemberRef {
    cast_lib: 0,
    cast_member: 0,
};

pub fn cast_member_ref(cast_lib: i32, cast_member: i32) -> CastMemberRef {
    CastMemberRef {
        cast_lib,
        cast_member,
    }
}

impl CastMemberRef {
    pub fn is_valid(&self) -> bool {
        self.cast_lib != INVALID_CAST_MEMBER_REF.cast_lib
            && self.cast_member != INVALID_CAST_MEMBER_REF.cast_member
    }
}

pub async fn player_cast_lib_set_prop(
    cast_lib: u32,
    prop_name: Symbol,
    value: Datum,
) -> Result<(), ScriptError> {
    let player = unsafe { PLAYER_OPT.as_mut().unwrap() };
    let builtin_prop_name = prop_name.into_builtin_or_error()?;

    let cast_manager = &mut player.movie.cast_manager;
    let cast_lib_obj = cast_manager.get_cast_mut(cast_lib as u32);

    if builtin_prop_name == BuiltInSymbol::FileName {
        // log::debug (not console) — the cast pool sets fileName on ~650 empty
        // slots during resetCastLibs; a per-set console.log floods DevTools and
        // dominates load time when the console is open.
        log::debug!(
            "Setting fileName of castLib {} ('{}') to '{}'",
            cast_lib,
            cast_lib_obj.name,
            value.string_value().unwrap_or_default()
        );
    }

    cast_lib_obj.set_prop(prop_name, value, &player.allocator)?;
    if builtin_prop_name == BuiltInSymbol::FileName {
        cast_lib_obj
            .preload(
                &mut player.net_manager,
                &mut player.bitmap_manager,
                &mut player.dir_cache,
            )
            .await;
    }
    // TODO handle preload error
    Ok(())
}

#[cfg(test)]
mod name_index_tests {
    use super::*;
    use crate::player::cast_member::FieldMember;

    fn empty_cast() -> CastLib {
        // Constructing cast members can format BuiltInSymbols, which read the
        // global symbol table; make sure it's initialized (idempotent).
        crate::player::symbols::symbol_table::init_symbol_table();
        CastLib {
            name: String::new(),
            file_name: String::new(),
            number: 1,
            is_external: false,
            state: CastLibState::Loaded,
            lctx: None,
            members: FxHashMap::default(),
            scripts: FxHashMap::default(),
            name_symbols: Vec::new(),
            preload_mode: 0,
            capital_x: false,
            dir_version: 0,
            palette_id_offset: 0,
            name_index: RefCell::new(None),
            font_table: HashMap::new(),
        }
    }

    fn field_member(number: u32, name: &str) -> CastMember {
        // Field members take the simple insert path (no JsApi / script wiring).
        let mut m = CastMember::new(number, CastMemberType::Field(FieldMember::new()));
        m.name = name.to_string();
        m
    }

    #[test]
    fn find_by_name_is_case_insensitive_and_lowest_number_wins() {
        let mut cast = empty_cast();
        cast.insert_member(5, field_member(5, "Window"));
        cast.insert_member(2, field_member(2, "window")); // duplicate name, lower number
        cast.insert_member(9, field_member(9, "Frame"));

        // Lowest-numbered match wins; lookup is case-insensitive.
        assert_eq!(cast.find_member_by_name("WINDOW").map(|m| m.number), Some(2));
        assert_eq!(cast.find_member_by_name("window").map(|m| m.number), Some(2));
        assert_eq!(cast.find_member_by_name("frame").map(|m| m.number), Some(9));
        assert!(cast.find_member_by_name("missing").is_none());
        // Second lookup hits the cached index and returns the same result.
        assert_eq!(cast.find_member_by_name("window").map(|m| m.number), Some(2));
    }

    #[test]
    fn index_invalidates_on_rename_and_remove() {
        let mut cast = empty_cast();
        cast.insert_member(3, field_member(3, "Alpha"));
        assert_eq!(cast.find_member_by_name("alpha").map(|m| m.number), Some(3)); // builds index

        // Rename: mutate the member then invalidate (mirrors the name-setter,
        // which routes through invalidate_member_name_cache -> invalidate_name_index).
        cast.members.get_mut(&3).unwrap().name = "Beta".to_string();
        cast.invalidate_name_index();
        assert!(cast.find_member_by_name("alpha").is_none());
        assert_eq!(cast.find_member_by_name("beta").map(|m| m.number), Some(3));

        // Remove via the map + invalidate (avoid remove_member's JsApi calls in tests).
        cast.members.remove(&3);
        cast.invalidate_name_index();
        assert!(cast.find_member_by_name("beta").is_none());
    }
}
