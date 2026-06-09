use std::{cell::RefCell, collections::HashMap, rc::Rc};

use fxhash::FxHashMap;
use url::Url;

use crate::{
    director::{
        cast::CastDef, chunks::sound::SoundChunk, enums::{ScriptType, BitmapInfo, SoundInfo},
        file::{read_director_file_bytes, DirectorFile},
        lingo::{datum::Datum, script::ScriptContext},
    },
    js_api::JsApi,
    utils::{get_base_url, get_basename_no_extension, log_i},
    player::{cast_member::ScriptMember, ci_string::CiString},
};

use super::{
    allocator::DatumAllocator,
    bitmap::{
        bitmap::{Bitmap, BuiltInPalette, PaletteRef},
        manager::BitmapManager,
    },
    cast_member::{
        BitmapMember, CastMember, CastMemberType, FieldMember, PaletteMember, SoundMember,
        TextMember, VectorShapeMember,
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
    pub preload_mode: u16,
    pub capital_x: bool,
    pub dir_version: u16,
    /// Offset to adjust bitmap clutId from Config-based to MCsL-based member numbering.
    pub palette_id_offset: i16,
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
            self.load_from_dir_file(cached_file, &file_name, bitmap_manager);
        } else {
            log_i(
                format_args!("Loading cast {} into castLib {} ('{}')", self.file_name, self.number, self.name)
                    .to_string()
                    .as_str(),
            );
            self.state = CastLibState::Loading;
            let task_id = net_manager.preload_net_thing(self.file_name.clone());
            if !net_manager.is_task_done(Some(task_id)) {
                net_manager.await_task(task_id).await;
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
            let cast_file = read_director_file_bytes(
                cast_bytes,
                &resolved_url.to_string(),
                &get_base_url(resolved_url).to_string(),
            );
            if let Ok(cast_file) = cast_file {
                dir_cache.insert(load_file_name.into(), cast_file);
                let cast_file = dir_cache.get(load_file_name).unwrap();
                self.load_from_dir_file(&cast_file, load_file_name, bitmap_manager);
                // We return here because the function `load_from_dir_file()`
                // has changed our `state` to `Loaded` and we want to keep this.
                return;
            } else {
                log_i(format!("Could not parse {load_file_name}").as_str());
            }
        } else {
            log_i(format!("Fetching {load_file_name} failed").as_str());
        }
        self.state = CastLibState::None;
    }

    pub fn find_member_by_number(&self, number: u32) -> Option<&CastMember> {
        self.members.get(&number)
    }

    pub fn find_mut_member_by_number(&mut self, number: u32) -> Option<&mut CastMember> {
        self.members.get_mut(&number)
    }

    pub fn find_member_by_name(&self, name: &str) -> Option<&CastMember> {
        // Director returns the lowest-numbered member when duplicates exist in the same cast.
        // HashMap iteration order is non-deterministic, so we must track the best match.
        let mut best: Option<&CastMember> = None;
        for member in self.members.values() {
            if member.name.eq_ignore_ascii_case(name) {
                if best.is_none() || member.number < best.unwrap().number {
                    best = Some(member);
                }
            }
        }
        best
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
        prop: &str,
        value: Datum,
        datums: &DatumAllocator,
    ) -> Result<(), ScriptError> {
        // TODO
        match prop {
            "preloadMode" => {
                self.preload_mode = value.int_value()? as u16;
            }
            "name" => {
                self.set_name(value.string_value()?);
            }
            "fileName" => {
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

    pub fn get_prop(&self, prop: &str) -> Result<Datum, ScriptError> {
        match prop {
            "preloadMode" => Ok(Datum::Int(self.preload_mode as i32)),
            "fileName" => {
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
            "number" => Ok(Datum::Int(self.number as i32)),
            "name" => Ok(Datum::String(self.name.clone())),
            "number of castMembers" | "number of members" => {
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
            log_i(
                format_args!(
                    "Applying cast def to castLib {} ('{}'): {} members",
                    self.number,
                    self.name,
                    cast_def.members.len()
                )
                .to_string()
                .as_str(),
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
        // Which lctx script should we register under this member's slot, and as
        // what type? A `Script` cast member registers its own script. A non-Script
        // member (Field, Text, Bitmap, Button, Shape) may carry an ATTACHED script
        // via `member_info.header.script_id` — Director lets `script("name")` and
        // `new(script(...))` resolve to that attached script, and a movie can store
        // a parent script AS a Field cast member (SpongeBob "JellyFishin'" stores
        // its "hero parent" parent script as a Field). Register it at the member
        // slot so `get_script_for_member(number)` finds it. (Member BEHAVIOR scripts
        // for mouse events are dispatched separately via get_behavior_script_from_lctx.)
        let registration: Option<(u32, ScriptType)> = match &member.member_type {
            CastMemberType::Script(s) => Some((s.script_id, s.script_type)),
            _ => member.get_script_id().map(|sid| (sid, ScriptType::Parent)),
        };

        if let Some((reg_script_id, reg_script_type)) = registration {
            let script_def = self
                .lctx
                .as_ref()
                .and_then(|lctx| lctx.scripts.get(&reg_script_id));

            if let Some(script_def) = script_def {
                let mut handler_names = Vec::new();
                let mut handler_name_map = FxHashMap::default();
                let names = &self.lctx.as_ref().unwrap().names;
                for (idx, handler) in script_def.handlers.iter().enumerate() {
                    // name_id 0xFFFF (and any out-of-range id) marks an
                    // anonymous handler slot — Director leaves these in the
                    // handler vector (netjack D4 has one). It must still occupy
                    // its position: `LocalCall` / `get_own_handler_ref_at`
                    // index handler_names BY POSITION, so skipping would
                    // misalign every later handler. Give it a unique synthetic
                    // name (registered in the map too) so it stays callable by
                    // index without indexing past the names table.
                    let handler_name = match names.get(handler.name_id as usize) {
                        Some(n) => n.clone(),
                        None => format!("__anon_handler_{}", idx),
                    };
                    handler_name_map.insert(CiString::from(handler_name.clone()), Rc::new(handler.clone()));
                    handler_names.push(handler_name);
                }

                let property_names = script_def
                    .property_name_ids
                    .iter()
                    .filter_map(|id| names.get(*id as usize).map(|n| n.to_owned()));
                let mut properties = FxHashMap::default();
                for name in property_names {
                    properties.insert(CiString::from(name), DatumRef::Void);
                }

                let script = Script {
                    member_ref: cast_member_ref(self.number as i32, number as i32),
                    name: (&member.name).to_owned(),
                    chunk: script_def.clone(),
                    script_type: reg_script_type,
                    handlers: handler_name_map,
                    handler_names,
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
    }

    pub fn create_member_at(
        &mut self,
        number: u32,
        member_type: &str,
        bitmap_manager: &mut BitmapManager,
    ) -> Result<CastMemberRef, ScriptError> {
        // Director symbols are case-insensitive (`new(#vectorShape)` ==
        // `new(#vectorshape)`), so match member-type names through `match_ci!`.
        let member = match_ci!(member_type, {
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
            },
            "palette" => Ok(CastMember::new(
                number,
                CastMemberType::Palette(PaletteMember::new()),
            )),
            // `new(#vectorShape)` creates an empty vector shape; the script
            // populates vertices via `addVertex` and sets fill/stroke/gradient
            // props (Director 11.5 Scripting Dictionary). spectral-wizard's
            // parent_grad builds gradient backgrounds this way at runtime.
            "vectorshape" => Ok(CastMember::new(
                number,
                CastMemberType::VectorShape(VectorShapeMember::new()),
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
        })?;
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
        let mut handler_name_map = FxHashMap::default();
        let names = &self.lctx.as_ref().unwrap().names;
        for (idx, handler) in script_chunk.handlers.iter().enumerate() {
            // Anonymous handler slots (name_id 0xFFFF / out of range) must keep
            // their position — LocalCall indexes handler_names by position.
            // Give them a unique synthetic name instead of skipping.
            let handler_name = match names.get(handler.name_id as usize) {
                Some(n) => n.clone(),
                None => format!("__anon_handler_{}", idx),
            };
            handler_name_map.insert(CiString::from(handler_name.clone()), Rc::new(handler.clone()));
            handler_names.push(handler_name);
        }

        // Build properties
        let property_names = script_chunk
            .property_name_ids
            .iter()
            .filter_map(|id| names.get(*id as usize).map(|n| n.to_owned()));
        let mut properties = FxHashMap::default();
        for name in property_names {
            properties.insert(CiString::from(name), DatumRef::Void);
        }

        let script = Rc::new(Script {
            member_ref: cast_member_ref(self.number as i32, cache_key as i32),
            name: format!("BehaviorScript_{}", script_id),
            chunk: script_chunk.clone(),
            script_type: ScriptType::Member,
            handlers: handler_name_map,
            handler_names,
            properties: RefCell::new(properties),
        });
        
        // Cache it with the offset key
        self.scripts.insert(cache_key, script.clone());
        
        Some(script)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
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
    prop_name: &str,
    value: Datum,
) -> Result<(), ScriptError> {
    let player = unsafe { PLAYER_OPT.as_mut().unwrap() };

    let cast_manager = &mut player.movie.cast_manager;
    let cast_lib_obj = cast_manager.get_cast_mut(cast_lib as u32);

    if prop_name == "fileName" {
        log_i(
            format_args!(
                "Setting fileName of castLib {} ('{}') to '{}'",
                cast_lib,
                cast_lib_obj.name,
                value.string_value().unwrap_or_default()
            )
            .to_string()
            .as_str(),
        );
    }

    cast_lib_obj.set_prop(&prop_name, value, &player.allocator)?;
    if prop_name == "fileName" {
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
