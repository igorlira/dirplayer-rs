use std::{cell::RefCell, collections::HashMap, rc::Rc};

use fxhash::FxHashMap;
use url::Url;

use crate::{
    director::{
        cast::CastDef,
        file::{read_director_file_bytes, DirectorFile},
        lingo::{datum::Datum, script::ScriptContext},
    },
    js_api::{self, JsApi},
    utils::{get_base_url, get_basename_no_extension, log_i},
};

use super::{
    allocator::DatumAllocator,
    bitmap::{
        bitmap::{Bitmap, BuiltInPalette, PaletteRef},
        manager::BitmapManager,
    },
    cast_member::{
        BitmapMember, CastMember, CastMemberType, FieldMember, PaletteMember, TextMember,
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
#[derive(PartialEq)]
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
    pub preload_mode: u8,
    pub capital_x: bool,
    pub dir_version: u16,
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
                format_args!("Loading cast {}", self.file_name)
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

    pub fn find_member_by_name(&self, name: &String) -> Option<&CastMember> {
        for member in self.members.values() {
            if member.name.eq_ignore_ascii_case(name) {
                return Some(member);
            }
        }
        None
    }

    fn clear(&mut self) {
        if self.state != CastLibState::Loaded {
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
        prop: &String,
        value: Datum,
        datums: &DatumAllocator,
    ) -> Result<(), ScriptError> {
        // TODO
        match prop.as_str() {
            "preloadMode" => {
                self.preload_mode = value.int_value()? as u8;
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

    pub fn get_prop(&self, prop: &String) -> Result<Datum, ScriptError> {
        match prop.as_str() {
            "preloadMode" => Ok(Datum::Int(self.preload_mode as i32)),
            "fileName" => Ok(Datum::String(self.file_name.clone())),
            "number" => Ok(Datum::Int(self.number as i32)),
            "name" => Ok(Datum::String(self.name.clone())),
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
            self.apply_cast_def(file, cast_def, bitmap_manager);
        }
    }

    pub fn apply_cast_def(
        &mut self,
        _: &DirectorFile,
        cast_def: &CastDef,
        bitmap_manager: &mut BitmapManager,
    ) {
        self.lctx = cast_def.lctx.clone();
        self.capital_x = cast_def.capital_x;
        self.dir_version = cast_def.dir_version;
        for (id, member_def) in &cast_def.members {
            self.insert_member(
                *id,
                CastMember::from(self.number, *id, member_def, &self.lctx, bitmap_manager),
            );
            JsApi::on_cast_member_name_changed(CastMemberRefHandlers::get_cast_slot_number(
                self.number,
                *id,
            ));
        }
        JsApi::dispatch_cast_member_list_changed(self.number);
        unsafe {
            PLAYER_OPT
                .as_mut()
                .unwrap()
                .movie
                .cast_manager
                .clear_movie_script_cache()
        };
    }

    pub fn insert_member(&mut self, number: u32, member: CastMember) {
        if let CastMemberType::Script(script_member) = &member.member_type {
            let script_def = self
                .lctx
                .as_ref()
                .unwrap()
                .scripts
                .get(&script_member.script_id)
                .unwrap();

            let mut handler_names = Vec::new();
            let mut handler_name_map = FxHashMap::default();
            for handler in &script_def.handlers {
                let handler_name = &self.lctx.as_ref().unwrap().names[handler.name_id as usize];
                handler_name_map.insert(handler_name.to_lowercase(), Rc::new(handler.clone()));
                handler_names.push(handler_name.to_owned());
            }

            let property_names = script_def
                .property_name_ids
                .iter()
                .map(|id| self.lctx.as_ref().unwrap().names[*id as usize].to_owned());
            let mut properties = FxHashMap::default();
            for name in property_names {
                properties.insert(name.clone(), DatumRef::Void);
            }

            let script = Script {
                member_ref: cast_member_ref(self.number as i32, number as i32),
                name: (&member.name).to_owned(),
                chunk: script_def.clone(),
                script_type: script_member.script_type,
                handlers: handler_name_map,
                handler_names,
                properties: RefCell::new(properties),
            };
            self.scripts.insert(number, Rc::new(script));
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
                    }),
                ))
            }
            "palette" => Ok(CastMember::new(
                number,
                CastMemberType::Palette(PaletteMember::new()),
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
        self.scripts.get(&number)
    }
}

#[derive(Clone, Debug, PartialEq)]
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
    prop_name: &String,
    value: Datum,
) -> Result<(), ScriptError> {
    let player = unsafe { PLAYER_OPT.as_mut().unwrap() };

    let cast_manager = &mut player.movie.cast_manager;
    let cast_lib = cast_manager.get_cast_mut(cast_lib as u32);
    cast_lib.set_prop(&prop_name, value, &player.allocator)?;
    if prop_name == "fileName" {
        cast_lib
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
