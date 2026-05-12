use std::{collections::HashMap, iter::FromIterator};

use itertools::Itertools;
use js_sys::Array;
use log::debug;
use wasm_bindgen::prelude::*;

use crate::{
    director::{
        chunks::{self, script::ScriptChunk, Chunk},
        enums::ScriptType,
        file::{DirectorFile, get_variable_multiplier},
        lingo::{datum::Datum, decompiler, script::ScriptContext},
        rifx::RIFXReaderContext,
        utils::fourcc_to_string,
    },
    player::{
        allocator::ScriptInstanceAllocatorTrait,
        bitmap::bitmap::PaletteRef,
        cast_lib::CastMemberRef,
        cast_member::{CastMember, CastMemberType, ScriptMember},
        datum_formatting::{format_concrete_datum, format_datum, format_float_with_precision, format_numeric_value},
        datum_ref::{DatumId, DatumRef},
        handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers,
        score::get_channel_number_from_index,
        score::Score,
        script::ScriptInstanceId,
        script_ref::ScriptInstanceRef,
        DirPlayer, ScriptError, PLAYER_OPT, sprite::{ColorRef, CursorRef},
    },
    rendering::RENDERER_LOCK,
};

#[derive(Clone)]
pub struct ScoreSpriteSpan {
    pub channel_number: u16,
    pub start_frame: u32,
    pub end_frame: u32,
    pub member_ref: [u16; 2], // [cast_lib, cast_member]
}

impl ToJsValue for ScoreSpriteSpan {
    fn to_js_value(&self) -> JsValue {
        let span_map = js_sys::Map::new();
        span_map.str_set("startFrame", &self.start_frame.to_js_value());
        span_map.str_set("endFrame", &self.end_frame.to_js_value());
        span_map.str_set("channelNumber", &self.channel_number.to_js_value());
        span_map.to_js_object().into()
    }
}

fn format_atom_summary(a: &crate::player::js_lingo::xdr::JsAtom) -> String {
    use crate::player::js_lingo::xdr::JsAtom;
    match a {
        JsAtom::Null => "null".into(),
        JsAtom::Void => "void".into(),
        JsAtom::Bool(b) => b.to_string(),
        JsAtom::Int(i) => i.to_string(),
        JsAtom::Double(d) => format!("{}", d),
        JsAtom::String(s) => format!("{:?}", s),
        JsAtom::Function(f) => format!(
            "function {}({})",
            f.name.as_deref().unwrap_or("<anonymous>"),
            f.bindings.iter()
                .filter(|b| b.kind == crate::player::js_lingo::xdr::JsBindingKind::Argument)
                .map(|b| b.name.as_str()).collect::<Vec<_>>().join(", ")
        ),
        JsAtom::Unsupported(t) => format!("<unsupported tag={}>", t),
    }
}

pub fn ascii_safe(string: &str) -> String {
    string
        .chars()
        .map(|c| match c as u32 {
            9 => '\t',
            10 => '\n',
            13 => '\r',
            32..=126 => c,
            _ => '?',
        })
        .collect()
}

pub fn safe_string(s: &str) -> String {
    String::from_utf8_lossy(s.as_bytes()).into_owned()
}

#[cfg(target_arch = "wasm32")]
pub fn safe_js_string(s: &str) -> JsValue {
    JsValue::from_str(&safe_string(s))
}

#[cfg(not(target_arch = "wasm32"))]
pub fn safe_js_string(s: &str) -> JsValue {
    let _ = s;
    JsValue::NULL
}

#[wasm_bindgen(getter_with_clone)]
pub struct OnMovieLoadedCallbackData {
    pub version: u16,
    pub test_val: String,
}

#[wasm_bindgen(getter_with_clone)]
pub struct OnScriptErrorCallbackData {
    pub message: String,
    pub script_member_ref: Option<JsBridgeMemberRef>,
    pub handler_name: Option<String>,
    pub is_paused: bool,
}

impl Into<js_sys::Map> for OnScriptErrorCallbackData {
    fn into(self) -> js_sys::Map {
        let map = js_sys::Map::new();
        map.str_set("message", &safe_js_string(&self.message));
        if let Some(script_member_ref) = self.script_member_ref {
            map.str_set("script_member_ref", &script_member_ref.to_js_value());
        } else {
            map.str_set("script_member_ref", &JsValue::NULL);
        }
        if let Some(handler_name) = self.handler_name {
            map.str_set("handler_name", &safe_js_string(&handler_name));
        } else {
            map.str_set("handler_name", &JsValue::NULL);
        }
        map.str_set("is_paused", &JsValue::from_bool(self.is_paused));
        map
    }
}

#[derive(Clone)]
#[wasm_bindgen(getter_with_clone)]
pub struct JsBridgeBreakpoint {
    pub script_name: String,
    pub handler_name: String,
    pub bytecode_index: usize,
}

impl Into<js_sys::Map> for JsBridgeBreakpoint {
    fn into(self) -> js_sys::Map {
        let map = js_sys::Map::new();
        map.str_set("script_name", &safe_js_string(&self.script_name));
        map.str_set("handler_name", &safe_js_string(&self.handler_name));
        map.str_set("bytecode_index", &JsValue::from(self.bytecode_index as u32));
        map
    }
}

pub type JsBridgeMemberRef = Vec<i32>;
pub type JsBridgeDatum = js_sys::Object;

pub struct JsBridgeScope {
    pub script_member_ref: JsBridgeMemberRef,
    pub bytecode_index: u32,
    pub handler_name: String,
    pub locals: HashMap<String, DatumRef>,
    pub stack: Vec<DatumRef>,
    pub args: Vec<DatumRef>,
}

impl Into<js_sys::Map> for JsBridgeScope {
    fn into(self) -> js_sys::Map {
        let map = js_sys::Map::new();
        map.str_set("script_member_ref", &self.script_member_ref.to_js_value());
        map.str_set("bytecode_index", &JsValue::from(self.bytecode_index));
        map.str_set("handler_name", &safe_js_string(&self.handler_name));

        let locals = js_sys::Map::new();
        for (k, v) in self.locals {
            locals.set(&safe_js_string(&k), &v.unwrap().to_js_value());
        }
        map.str_set("locals", &locals.to_js_object());

        let stack = js_sys::Array::new();
        for item in self.stack {
            stack.push(&item.unwrap().to_js_value());
        }
        map.str_set("stack", &stack);

        let args = js_sys::Array::new();
        for item in self.args {
            args.push(&item.unwrap().to_js_value());
        }
        map.str_set("args", &args);

        map
    }
}

impl ToJsValue for Vec<i32> {
    fn to_js_value(&self) -> JsValue {
        let array = js_sys::Array::new();
        for item in self {
            array.push(&JsValue::from_f64(*item as f64));
        }
        array.into()
    }
}

impl ToJsValue for Vec<u32> {
    fn to_js_value(&self) -> JsValue {
        let array = js_sys::Array::new();
        for item in self {
            array.push(&JsValue::from_f64(*item as f64));
        }
        array.into()
    }
}

impl ToJsValue for Vec<usize> {
    fn to_js_value(&self) -> JsValue {
        let array = js_sys::Array::new();
        for item in self {
            array.push(&JsValue::from_f64(*item as f64));
        }
        array.into()
    }
}

impl CastMemberRef {
    pub fn to_js(&self) -> JsBridgeMemberRef {
        vec![self.cast_lib, self.cast_member]
    }
}

#[wasm_bindgen(module = "dirplayer-js-api")]
extern "C" {
    pub fn onMovieLoaded(test: OnMovieLoadedCallbackData);
    pub fn onMovieLoadFailed(path: &str, error: &str);
    pub fn onCastListChanged(names: Array);
    pub fn onCastLibNameChanged(cast_number: u32, name: &str);
    pub fn onCastMemberListChanged(cast_number: u32, members: js_sys::Object);
    pub fn onCastMemberChanged(member_ref: JsValue, member: js_sys::Object);
    pub fn onScoreChanged(snapshot: js_sys::Object);
    pub fn onChannelChanged(channel: i16, snapshot: js_sys::Object);
    pub fn onChannelDisplayNameChanged(channel: i16, display_name: &str);
    pub fn onFrameChanged(frame: u32);
    pub fn onScriptError(data: js_sys::Object);
    pub fn onScopeListChanged(scopes: Vec<js_sys::Object>);
    pub fn onBreakpointListChanged(data: Vec<js_sys::Object>);
    pub fn onGlobalListChanged(data: js_sys::Object);
    pub fn onScriptErrorCleared();
    pub fn onDebugMessage(message: &str);
    pub fn onDebugContent(content: js_sys::Object);
    pub fn onScheduleTimeout(timeout_name: &str, interval: u32);
    pub fn onClearTimeout(timeout_name: &str);
    pub fn onClearTimeouts();
    pub fn onDatumSnapshot(datum_id: DatumId, data: js_sys::Object);
    pub fn onScriptInstanceSnapshot(script_ref: ScriptInstanceId, data: js_sys::Object);
    pub fn onExternalEvent(event: &str);
    pub fn onFlashMemberLoaded(cast_lib: i32, cast_member: i32, swf_data: &[u8], width: u32, height: u32);
    pub fn onFlashMemberUnloaded(cast_lib: i32, cast_member: i32);
    pub fn onStageSizeChanged(width: u32, height: u32, center: bool);
}

pub struct JsApi {}

#[cfg(target_arch = "wasm32")]
impl JsApi {
    pub fn dispatch_datum_snapshot(datum_ref: &DatumRef, player: &DirPlayer) {
        let snapshot = datum_to_js_bridge(datum_ref, player, 0);
        onDatumSnapshot(datum_ref.unwrap(), snapshot);
    }
    pub fn dispatch_script_instance_snapshot(
        script_ref: Option<ScriptInstanceRef>,
        player: &DirPlayer,
    ) {
        let datum = if script_ref.is_none() {
            Datum::Void
        } else {
            Datum::ScriptInstanceRef(script_ref.clone().unwrap())
        };
        let snapshot = concrete_datum_to_js_bridge(&datum, player, 0);
        onScriptInstanceSnapshot(*script_ref.unwrap(), snapshot);
    }
    pub fn dispatch_schedule_timeout(timeout_name: &str, interval: u32) {
        onScheduleTimeout(timeout_name, interval);
    }
    pub fn dispatch_clear_timeout(timeout_name: &str) {
        onClearTimeout(timeout_name);
    }
    #[allow(dead_code)]
    pub fn dispatch_clear_timeouts() {
        onClearTimeouts();
    }
    pub fn dispatch_flash_member_loaded(cast_lib: i32, cast_member: i32, swf_data: &[u8], width: u32, height: u32) {
        onFlashMemberLoaded(cast_lib, cast_member, swf_data, width, height);
    }
    pub fn dispatch_flash_member_unloaded(cast_lib: i32, cast_member: i32) {
        onFlashMemberUnloaded(cast_lib, cast_member);
    }
    pub fn dispatch_stage_size_changed(width: u32, height: u32, center: bool) {
        onStageSizeChanged(width, height, center);
    }
    pub fn dispatch_movie_loaded(dir_file: &DirectorFile) {
        let test = dir_file
            .cast_entries
            .iter()
            .map(|cast| cast.name.to_owned())
            .collect_vec()
            .join(", ");

        onMovieLoaded(OnMovieLoadedCallbackData {
            version: dir_file.version,
            test_val: test,
        });
    }

    pub fn dispatch_movie_load_failed(path: &str, error: &str) {
        onMovieLoadFailed(path, error);
    }

    /// Collects all chunk IDs that are transitive descendants of `root_id` in the KeyTable,
    /// plus root_id itself. This walks the parent→children relationship recursively:
    /// KeyTable entries map section_id (child) → cast_id (parent).
    fn collect_cast_descendants(
        root_id: u32,
        children_map: &HashMap<u32, Vec<u32>>,
    ) -> std::collections::HashSet<u32> {
        let mut result = std::collections::HashSet::new();
        let mut stack = vec![root_id];
        while let Some(id) = stack.pop() {
            if result.insert(id) {
                if let Some(children) = children_map.get(&id) {
                    for child in children {
                        stack.push(*child);
                    }
                }
            }
        }
        result
    }

    /// Builds a parent→children adjacency map from the KeyTable.
    fn build_children_map(dir_file: &DirectorFile) -> HashMap<u32, Vec<u32>> {
        let mut children_map: HashMap<u32, Vec<u32>> = HashMap::new();
        if let Some(kt) = dir_file.key_table.as_ref() {
            for entry in kt.entries.iter().take(kt.used_count as usize) {
                children_map.entry(entry.cast_id).or_default().push(entry.section_id);
            }
        }
        children_map
    }

    pub fn get_cast_chunk_list_for(player: &DirPlayer, cast_number: u32) -> js_sys::Object {
        let result = js_sys::Map::new();

        let cast_lib = match player.movie.cast_manager.get_cast_or_null(cast_number) {
            Some(c) => c,
            None => return result.to_js_object(),
        };

        // Find the DirectorFile that contains the chunks
        let dir_file = if cast_lib.is_external {
            player.dir_cache.get(cast_lib.file_name.as_str())
        } else {
            player.movie.file.as_ref()
        };

        let dir_file = match dir_file {
            Some(f) => f,
            None => return result.to_js_object(),
        };

        // Find the CastDef that matches this cast
        let cast_def = if cast_lib.is_external {
            dir_file.casts.first()
        } else {
            let cast_entry = dir_file.cast_entries.get((cast_number as usize).wrapping_sub(1));
            cast_entry.and_then(|entry| {
                dir_file.casts.iter().find(|cd| cd.id == entry.id)
            })
        };

        let cast_def = match cast_def {
            Some(cd) => cd,
            None => return result.to_js_object(),
        };

        let chunk_container = &dir_file.chunk_container;
        let key_table = dir_file.key_table.as_ref();

        // Build owner_map (child → parent) from KeyTable
        let owner_map: HashMap<u32, u32> = key_table
            .map(|kt| {
                kt.entries.iter()
                    .take(kt.used_count as usize)
                    .map(|e| (e.section_id, e.cast_id))
                    .collect()
            })
            .unwrap_or_default();

        // Build parent → children map and collect ALL transitive descendants of the cast root.
        // This captures structural chunks like Lctx → Lscr, Lnam, etc.
        let children_map = Self::build_children_map(dir_file);
        let mut cast_chunk_ids = Self::collect_cast_descendants(cast_def.id, &children_map);

        // Also include all chunks from section_to_member (CASt member chunks and their media
        // children). These are referenced from the CAS* member_ids array, not through the
        // KeyTable parent-child chain, so collect_cast_descendants doesn't find them.
        for section_id in cast_def.section_to_member.keys() {
            cast_chunk_ids.insert(*section_id);
        }

        // Also include Lscr and Lnam chunks referenced internally by the Lctx chunk.
        // These are NOT in the KeyTable as children of Lctx, so neither
        // collect_cast_descendants nor section_to_member finds them.
        for section_id in &cast_def.lctx_child_section_ids {
            cast_chunk_ids.insert(*section_id);
        }

        // Emit all chunks that belong to this cast
        for chunk_id in &cast_chunk_ids {
            let chunk_info = match chunk_container.chunk_info.get(chunk_id) {
                Some(ci) => ci,
                None => continue,
            };

            let fourcc_str = fourcc_to_string(chunk_info.fourcc);
            let chunk_map = js_sys::Map::new();
            chunk_map.str_set("id", &JsValue::from_f64(*chunk_id as f64));
            chunk_map.str_set("fourcc", &safe_js_string(&fourcc_str));
            chunk_map.str_set("len", &JsValue::from_f64(chunk_info.len as f64));
            chunk_map.str_set("castLib", &JsValue::from_f64(cast_number as f64));

            if let Some(owner_id) = owner_map.get(chunk_id) {
                chunk_map.str_set("owner", &JsValue::from_f64(*owner_id as f64));
            } else if cast_def.lctx_child_section_ids.contains(chunk_id) {
                // Lscr/Lnam chunks aren't in the KeyTable, so set their owner
                // to the Lctx section_id so they appear under it in the tree view.
                if let Some(lctx_sid) = cast_def.lctx_section_id {
                    chunk_map.str_set("owner", &JsValue::from_f64(lctx_sid as f64));
                }
            }

            // Annotate with member info if this chunk belongs to a specific member
            if let Some((member_number, member_name)) = cast_def.section_to_member.get(chunk_id) {
                chunk_map.str_set("memberNumber", &JsValue::from_f64(*member_number as f64));
                chunk_map.str_set("memberName", &safe_js_string(member_name));
            }

            result.set(
                &JsValue::from_f64(*chunk_id as f64),
                &chunk_map.to_js_object(),
            );
        }

        result.to_js_object()
    }

    /// Returns all chunks from the main movie file that are NOT associated with any cast.
    pub fn get_movie_top_level_chunks(player: &DirPlayer) -> js_sys::Object {
        let result = js_sys::Map::new();

        let dir_file = match player.movie.file.as_ref() {
            Some(f) => f,
            None => return result.to_js_object(),
        };

        let chunk_container = &dir_file.chunk_container;
        let key_table = dir_file.key_table.as_ref();

        // Build parent → children map and collect ALL transitive descendants of every cast root
        let children_map = Self::build_children_map(dir_file);
        let mut cast_section_ids = std::collections::HashSet::new();
        for cast_def in &dir_file.casts {
            let descendants = Self::collect_cast_descendants(cast_def.id, &children_map);
            for id in descendants {
                cast_section_ids.insert(id);
            }
            // Also exclude chunks belonging to cast members (CASt chunks and their media
            // children). These are referenced from the CAS* member_ids array, not through
            // the KeyTable parent-child chain, so collect_cast_descendants doesn't find them.
            for section_id in cast_def.section_to_member.keys() {
                cast_section_ids.insert(*section_id);
            }
            // Also exclude Lscr and Lnam chunks referenced internally by Lctx.
            for section_id in &cast_def.lctx_child_section_ids {
                cast_section_ids.insert(*section_id);
            }
        }

        // Build owner_map from KeyTable
        let owner_map: HashMap<u32, u32> = key_table
            .map(|kt| {
                kt.entries.iter()
                    .take(kt.used_count as usize)
                    .map(|e| (e.section_id, e.cast_id))
                    .collect()
            })
            .unwrap_or_default();

        for (chunk_id, chunk_info) in &chunk_container.chunk_info {
            if cast_section_ids.contains(chunk_id) {
                continue;
            }

            let fourcc_str = fourcc_to_string(chunk_info.fourcc);
            let chunk_map = js_sys::Map::new();
            chunk_map.str_set("id", &JsValue::from_f64(*chunk_id as f64));
            chunk_map.str_set("fourcc", &safe_js_string(&fourcc_str));
            chunk_map.str_set("len", &JsValue::from_f64(chunk_info.len as f64));

            if let Some(owner_id) = owner_map.get(chunk_id) {
                chunk_map.str_set("owner", &JsValue::from_f64(*owner_id as f64));
            }

            result.set(
                &JsValue::from_f64(*chunk_id as f64),
                &chunk_map.to_js_object(),
            );
        }

        result.to_js_object()
    }

    /// Returns the raw bytes of a chunk by ID from the specified cast's DirectorFile.
    /// If cast_number is 0, uses the main movie file.
    pub fn get_chunk_bytes(player: &DirPlayer, cast_number: u32, chunk_id: u32) -> Option<Vec<u8>> {
        let dir_file = if cast_number == 0 {
            player.movie.file.as_ref()
        } else {
            let cast_lib = player.movie.cast_manager.get_cast_or_null(cast_number)?;
            if cast_lib.is_external {
                player.dir_cache.get(cast_lib.file_name.as_str())
            } else {
                player.movie.file.as_ref()
            }
        };

        let dir_file = dir_file?;
        dir_file.chunk_container.cached_chunk_views.get(&chunk_id).cloned()
    }

    fn chunk_to_js(chunk: &Chunk) -> js_sys::Object {
        let map = js_sys::Map::new();
        match chunk {
            Chunk::Cast(c) => {
                map.str_set("type", &JsValue::from_str("CAS*"));
                let ids = js_sys::Array::new();
                for id in &c.member_ids {
                    ids.push(&JsValue::from_f64(*id as f64));
                }
                map.str_set("member_ids", &ids);
            }
            Chunk::CastMember(c) => {
                map.str_set("type", &JsValue::from_str("CASt"));
                map.str_set("member_type", &JsValue::from_str(&format!("{:?}", c.member_type)));
                if let Some(info) = &c.member_info {
                    map.str_set("name", &JsValue::from_str(&ascii_safe(&info.name)));
                    if !info.script_src_text.is_empty() {
                        map.str_set("script_src_text", &JsValue::from_str(&ascii_safe(&info.script_src_text)));
                    }
                    map.str_set("script_id", &JsValue::from_f64(info.header.script_id as f64));
                    map.str_set("flags", &JsValue::from_f64(info.header.flags as f64));
                }
                // Serialize type-specific data
                match &c.specific_data {
                    crate::director::chunks::cast_member::CastMemberSpecificData::Script(st) => {
                        map.str_set("script_type", &JsValue::from_str(&format!("{:?}", st)));
                    }
                    crate::director::chunks::cast_member::CastMemberSpecificData::Bitmap(bi) => {
                        let bm = js_sys::Map::new();
                        bm.str_set("width", &JsValue::from_f64(bi.width as f64));
                        bm.str_set("height", &JsValue::from_f64(bi.height as f64));
                        bm.str_set("reg_x", &JsValue::from_f64(bi.reg_x as f64));
                        bm.str_set("reg_y", &JsValue::from_f64(bi.reg_y as f64));
                        bm.str_set("bit_depth", &JsValue::from_f64(bi.bit_depth as f64));
                        bm.str_set("palette_id", &JsValue::from_f64(bi.palette_id as f64));
                        map.str_set("bitmap_info", &bm.to_js_object());
                    }
                    crate::director::chunks::cast_member::CastMemberSpecificData::Text(ti) => {
                        let tm = js_sys::Map::new();
                        tm.str_set("width", &JsValue::from_f64(ti.width as f64));
                        tm.str_set("height", &JsValue::from_f64(ti.height as f64));
                        tm.str_set("editable", &JsValue::from_bool(ti.editable));
                        tm.str_set("box_type", &JsValue::from_f64(ti.box_type as f64));
                        tm.str_set("anti_alias", &JsValue::from_bool(ti.anti_alias));
                        map.str_set("text_info", &tm.to_js_object());
                    }
                    crate::director::chunks::cast_member::CastMemberSpecificData::Field(fi) => {
                        let fm = js_sys::Map::new();
                        fm.str_set("alignment", &JsValue::from_f64(fi.alignment as f64));
                        map.str_set("field_info", &fm.to_js_object());
                    }
                    _ => {}
                }
            }
            Chunk::CastList(c) => {
                map.str_set("type", &JsValue::from_str("MCsL"));
                let entries = js_sys::Array::new();
                for entry in &c.entries {
                    let em = js_sys::Map::new();
                    em.str_set("name", &JsValue::from_str(&ascii_safe(&entry.name)));
                    em.str_set("file_path", &JsValue::from_str(&ascii_safe(&entry.file_path)));
                    em.str_set("id", &JsValue::from_f64(entry.id as f64));
                    em.str_set("min_member", &JsValue::from_f64(entry.min_member as f64));
                    em.str_set("max_member", &JsValue::from_f64(entry.max_member as f64));
                    em.str_set("preload_settings", &JsValue::from_f64(entry.preload_settings as f64));
                    entries.push(&em.to_js_object());
                }
                map.str_set("entries", &entries);
            }
            Chunk::KeyTable(kt) => {
                map.str_set("type", &JsValue::from_str("KEY*"));
                map.str_set("used_count", &JsValue::from_f64(kt.used_count as f64));
                map.str_set("entry_count", &JsValue::from_f64(kt.entry_count as f64));
                let entries = js_sys::Array::new();
                for entry in kt.entries.iter().take(kt.used_count as usize) {
                    let em = js_sys::Map::new();
                    em.str_set("section_id", &JsValue::from_f64(entry.section_id as f64));
                    em.str_set("cast_id", &JsValue::from_f64(entry.cast_id as f64));
                    em.str_set("fourcc", &JsValue::from_str(&fourcc_to_string(entry.fourcc)));
                    entries.push(&em.to_js_object());
                }
                map.str_set("entries", &entries);
            }
            Chunk::ScriptContext(sc) => {
                map.str_set("type", &JsValue::from_str("Lctx"));
                map.str_set("entry_count", &JsValue::from_f64(sc.entry_count as f64));
                map.str_set("lnam_section_id", &JsValue::from_f64(sc.lnam_section_id as f64));
                let entries = js_sys::Array::new();
                for entry in &sc.section_map {
                    let em = js_sys::Map::new();
                    em.str_set("section_id", &JsValue::from_f64(entry.section_id as f64));
                    entries.push(&em.to_js_object());
                }
                map.str_set("section_map", &entries);
            }
            Chunk::ScriptNames(sn) => {
                map.str_set("type", &JsValue::from_str("Lnam"));
                let names = js_sys::Array::new();
                for name in &sn.names {
                    names.push(&JsValue::from_str(&ascii_safe(name)));
                }
                map.str_set("names", &names);
            }
            Chunk::Script(sc) => {
                map.str_set("type", &JsValue::from_str("Lscr"));
                map.str_set("handler_count", &JsValue::from_f64(sc.handlers.len() as f64));
                map.str_set("literal_count", &JsValue::from_f64(sc.literals.len() as f64));
                let prop_ids = js_sys::Array::new();
                for id in &sc.property_name_ids {
                    prop_ids.push(&JsValue::from_f64(*id as f64));
                }
                map.str_set("property_name_ids", &prop_ids);
                let handlers = js_sys::Array::new();
                for handler in &sc.handlers {
                    let hm = js_sys::Map::new();
                    hm.str_set("name_id", &JsValue::from_f64(handler.name_id as f64));
                    hm.str_set("bytecode_count", &JsValue::from_f64(handler.bytecode_array.len() as f64));
                    let arg_ids = js_sys::Array::new();
                    for id in &handler.argument_name_ids {
                        arg_ids.push(&JsValue::from_f64(*id as f64));
                    }
                    hm.str_set("argument_name_ids", &arg_ids);
                    let local_ids = js_sys::Array::new();
                    for id in &handler.local_name_ids {
                        local_ids.push(&JsValue::from_f64(*id as f64));
                    }
                    hm.str_set("local_name_ids", &local_ids);
                    let global_ids = js_sys::Array::new();
                    for id in &handler.global_name_ids {
                        global_ids.push(&JsValue::from_f64(*id as f64));
                    }
                    hm.str_set("global_name_ids", &global_ids);
                    handlers.push(&hm.to_js_object());
                }
                map.str_set("handlers", &handlers);
                let literals = js_sys::Array::new();
                for literal in &sc.literals {
                    let lm = js_sys::Map::new();
                    lm.str_set("type", &JsValue::from_str(&literal.type_str()));
                    match literal {
                        Datum::Int(v) => {
                            lm.str_set("value", &JsValue::from_f64(*v as f64));
                        }
                        Datum::Float(v) => {
                            lm.str_set("value", &JsValue::from_f64(*v));
                        }
                        Datum::String(s) => {
                            lm.str_set("value", &JsValue::from_str(&ascii_safe(s)));
                        }
                        Datum::Symbol(s) => {
                            lm.str_set("value", &JsValue::from_str(&ascii_safe(s)));
                        }
                        Datum::JavaScript(data) => {
                            lm.str_set("size", &JsValue::from_f64(data.len() as f64));
                            lm.str_set("bytes", &js_sys::Uint8Array::from(&data[..]));
                        }
                        Datum::Void => {}
                        _ => {
                            lm.str_set("value", &JsValue::from_str(&literal.type_str()));
                        }
                    }
                    literals.push(&lm.to_js_object());
                }
                map.str_set("literals", &literals);
            }
            Chunk::Config(c) => {
                map.str_set("type", &JsValue::from_str("VWCF"));
                map.str_set("director_version", &JsValue::from_f64(c.director_version as f64));
                map.str_set("movie_top", &JsValue::from_f64(c.movie_top as f64));
                map.str_set("movie_left", &JsValue::from_f64(c.movie_left as f64));
                map.str_set("movie_bottom", &JsValue::from_f64(c.movie_bottom as f64));
                map.str_set("movie_right", &JsValue::from_f64(c.movie_right as f64));
                map.str_set("min_member", &JsValue::from_f64(c.min_member as f64));
                map.str_set("max_member", &JsValue::from_f64(c.max_member as f64));
                map.str_set("frame_rate", &JsValue::from_f64(c.frame_rate as f64));
                map.str_set("bit_depth", &JsValue::from_f64(c.bit_depth as f64));
                map.str_set("platform", &JsValue::from_f64(c.platform as f64));
            }
            Chunk::Text(t) => {
                map.str_set("type", &JsValue::from_str("STXT"));
                map.str_set("text", &JsValue::from_str(&ascii_safe(&t.text)));
                map.str_set("text_length", &JsValue::from_f64(t.text_length as f64));
                let runs = t.parse_formatting_runs();
                let runs_arr = js_sys::Array::new();
                for run in &runs {
                    let rm = js_sys::Map::new();
                    rm.str_set("start_position", &JsValue::from_f64(run.start_position as f64));
                    rm.str_set("font_id", &JsValue::from_f64(run.font_id as f64));
                    rm.str_set("font_size", &JsValue::from_f64(run.font_size as f64));
                    rm.str_set("style", &JsValue::from_f64(run.style as f64));
                    runs_arr.push(&rm.to_js_object());
                }
                map.str_set("formatting_runs", &runs_arr);
            }
            Chunk::Palette(p) => {
                map.str_set("type", &JsValue::from_str("CLUT"));
                let colors = js_sys::Array::new();
                for (r, g, b) in &p.colors {
                    colors.push(&JsValue::from_str(&format!("#{:02x}{:02x}{:02x}", r, g, b)));
                }
                map.str_set("colors", &colors);
            }
            Chunk::Sound(s) => {
                map.str_set("type", &JsValue::from_str("snd "));
                map.str_set("channels", &JsValue::from_f64(s.channels() as f64));
                map.str_set("sample_rate", &JsValue::from_f64(s.sample_rate() as f64));
                map.str_set("bits_per_sample", &JsValue::from_f64(s.bits_per_sample() as f64));
                map.str_set("sample_count", &JsValue::from_f64(s.sample_count() as f64));
                map.str_set("codec", &JsValue::from_str(&ascii_safe(&s.codec())));
                map.str_set("data_size", &JsValue::from_f64(s.data().len() as f64));
            }
            Chunk::Score(sc) => {
                map.str_set("type", &JsValue::from_str("VWSC"));
                map.str_set("entry_count", &JsValue::from_f64(sc.header.entry_count as f64));
                map.str_set("frame_interval_count", &JsValue::from_f64(sc.frame_intervals.len() as f64));
            }
            Chunk::FrameLabels(fl) => {
                map.str_set("type", &JsValue::from_str("VWLB"));
                let labels = js_sys::Array::new();
                for label in &fl.labels {
                    let lm = js_sys::Map::new();
                    lm.str_set("frame_num", &JsValue::from_f64(label.frame_num as f64));
                    lm.str_set("label", &JsValue::from_str(&ascii_safe(&label.label)));
                    labels.push(&lm.to_js_object());
                }
                map.str_set("labels", &labels);
            }
            Chunk::Bitmap(b) => {
                map.str_set("type", &JsValue::from_str("BITD"));
                map.str_set("data_size", &JsValue::from_f64(b.data.len() as f64));
                map.str_set("version", &JsValue::from_f64(b.version as f64));
            }
            Chunk::XMedia(xm) => {
                map.str_set("type", &JsValue::from_str("XMED"));
                map.str_set("data_size", &JsValue::from_f64(xm.raw_data.len() as f64));
                if xm.is_pfr_font() {
                    map.str_set("content_type", &JsValue::from_str("PFR1 Font"));
                    if let Some(font) = xm.parse_pfr_font() {
                        map.str_set("font_name", &JsValue::from_str(&ascii_safe(&font.font_name)));
                        map.str_set("outline_glyph_count", &JsValue::from_f64(font.parsed.glyphs.len() as f64));
                        map.str_set("bitmap_glyph_count", &JsValue::from_f64(font.parsed.bitmap_glyphs.len() as f64));
                        map.str_set("target_em_px", &JsValue::from_f64(font.parsed.target_em_px as f64));
                    }
                } else if xm.is_styled_text() {
                    map.str_set("content_type", &JsValue::from_str("Styled Text"));
                    if let Some(st) = xm.parse_styled_text() {
                        map.str_set("text", &JsValue::from_str(&ascii_safe(&st.text)));
                        map.str_set("alignment", &JsValue::from_str(&format!("{:?}", st.alignment)));
                        map.str_set("word_wrap", &JsValue::from_bool(st.word_wrap));
                        map.str_set("width", &JsValue::from_f64(st.width as f64));
                        map.str_set("height", &JsValue::from_f64(st.height as f64));
                        map.str_set("line_count", &JsValue::from_f64(st.line_count as f64));
                        map.str_set("fixed_line_space", &JsValue::from_f64(st.fixed_line_space as f64));
                        let spans = js_sys::Array::new();
                        for span in &st.styled_spans {
                            let sm = js_sys::Map::new();
                            sm.str_set("text", &JsValue::from_str(&ascii_safe(&span.text)));
                            if let Some(face) = &span.style.font_face {
                                sm.str_set("font_face", &JsValue::from_str(&ascii_safe(face)));
                            }
                            if let Some(size) = span.style.font_size {
                                sm.str_set("font_size", &JsValue::from_f64(size as f64));
                            }
                            sm.str_set("bold", &JsValue::from_bool(span.style.bold));
                            sm.str_set("italic", &JsValue::from_bool(span.style.italic));
                            sm.str_set("underline", &JsValue::from_bool(span.style.underline));
                            if let Some(color) = span.style.color {
                                sm.str_set("color", &JsValue::from_str(&format!("#{:06x}", color)));
                            }
                            spans.push(&sm.to_js_object());
                        }
                        map.str_set("styled_spans", &spans);
                    }
                } else {
                    map.str_set("content_type", &JsValue::from_str("Unknown"));
                }
            }
            _ => {
                map.str_set("type", &JsValue::from_str("unsupported"));
            }
        }
        map.to_js_object()
    }

    /// Returns parsed chunk data as a JS object. Re-parses from cached raw bytes on demand.
    pub fn get_parsed_chunk(player: &DirPlayer, cast_number: u32, chunk_id: u32) -> js_sys::Object {
        let error_result = |msg: &str| -> js_sys::Object {
            let map = js_sys::Map::new();
            map.str_set("error", &JsValue::from_str(&ascii_safe(msg)));
            map.to_js_object()
        };

        let dir_file = if cast_number == 0 {
            player.movie.file.as_ref()
        } else {
            let cast_lib = match player.movie.cast_manager.get_cast_or_null(cast_number) {
                Some(c) => c,
                None => return error_result("Cast not found"),
            };
            if cast_lib.is_external {
                player.dir_cache.get(cast_lib.file_name.as_str())
            } else {
                player.movie.file.as_ref()
            }
        };

        let dir_file = match dir_file {
            Some(f) => f,
            None => return error_result("DirectorFile not found"),
        };

        let chunk_info = match dir_file.chunk_container.chunk_info.get(&chunk_id) {
            Some(ci) => ci,
            None => return error_result("Chunk info not found"),
        };

        let raw_bytes = match dir_file.chunk_container.cached_chunk_views.get(&chunk_id) {
            Some(b) => b,
            None => return error_result("Raw bytes not found"),
        };

        // Determine lctx_capital_x for this chunk's cast
        let lctx_capital_x = dir_file.casts.iter().find(|cd| {
            cd.lctx_child_section_ids.contains(&chunk_id)
        }).map(|cd| cd.capital_x).unwrap_or(false);

        let mut rifx = RIFXReaderContext {
            after_burned: dir_file.after_burned,
            ils_body_offset: 0,
            dir_version: dir_file.version,
            lctx_capital_x,
        };

        match chunks::make_chunk(dir_file.endian, &mut rifx, chunk_info.fourcc, raw_bytes) {
            Ok(chunk) => Self::chunk_to_js(&chunk),
            Err(e) => error_result(&e),
        }
    }

    pub fn dispatch_cast_name_changed(cast_number: u32) {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
            let cast = player.movie.cast_manager.get_cast(cast_number).unwrap();
            onCastLibNameChanged(cast_number, &cast.name);
        });
    }

    pub fn dispatch_cast_list_changed() {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
            let names = player
                .movie
                .cast_manager
                .casts
                .iter()
                .map(|x| x.name.to_owned())
                .collect_vec();

            onCastListChanged(
                names
                    .into_iter()
                    .map(|x| safe_js_string(&x))
                    .collect::<Array>(),
            );
        });
    }

    pub fn dispatch_cast_member_list_changed(cast_number: u32) {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
            let cast = match player.movie.cast_manager.get_cast(cast_number) {
                Ok(cast) => cast,
                Err(_) => return,
            };
            let members_iter = cast.members.values().into_iter();

            let member_list = js_sys::Map::new();
            for member in members_iter {
                let member_map = Self::get_mini_member_snapshot(member);
                member_list.set(&JsValue::from(member.number), &member_map.to_js_object());
            }

            onCastMemberListChanged(cast_number, member_list.to_js_object());
        });
    }

    pub fn dispatch_cast_member_changed(member_ref: CastMemberRef) {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
            let subscribed_members = &player.subscribed_member_refs;
            if !subscribed_members.contains(&member_ref) {
                return;
            }

            let cast = player
                .movie
                .cast_manager
                .get_cast(member_ref.cast_lib as u32)
                .unwrap();
            let member = cast.members.get(&(member_ref.cast_member as u32)).unwrap();
            let member_map = Self::get_member_snapshot(member, member_ref.cast_lib as u32, cast.lctx.as_ref(), player);

            onCastMemberChanged(member_ref.to_js().to_js_value(), member_map.to_js_object());
        });
    }

    pub fn on_cast_member_name_changed(slot_number: u32) {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };

            if player.is_subscribed_to_channel_names {
                for channel in player.movie.score.channels.iter() {
                    if channel.sprite.member.as_ref().map(|x| {
                        CastMemberRefHandlers::get_cast_slot_number(
                            x.cast_lib as u32,
                            x.cast_member as u32,
                        )
                    }) == Some(slot_number)
                    {
                        Self::dispatch_channel_name_changed(channel.number as i16);
                    }
                }
            }
        });
    }

    pub fn on_sprite_member_changed(sprite_num: i16) {
        Self::dispatch_channel_name_changed(sprite_num)
    }

    pub fn dispatch_score_changed() {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };

            let snapshot = Self::get_score_snapshot(player, &player.movie.score);
            onScoreChanged(snapshot.to_js_object());
        });
    }

    pub fn dispatch_channel_changed(channel: i16) {
        let selected_channel = RENDERER_LOCK.with(|x| {
            let borrowed = x.borrow();
            let dynamic = borrowed.as_ref();
            // Check Canvas2D renderer first, then WebGL2
            dynamic
                .and_then(|d| d.as_canvas2d())
                .and_then(|canvas2d| canvas2d.debug_selected_channel_num)
                .or_else(|| {
                    dynamic
                        .and_then(|d| d.as_webgl2())
                        .and_then(|webgl2| webgl2.debug_selected_channel_num)
                })
        });

        if selected_channel == Some(channel) {
            async_std::task::spawn_local(async move {
                let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
                let snapshot = Self::get_channel_snapshot(player, &channel);
                onChannelChanged(channel, snapshot.to_js_object());
            });
        }
    }

    pub fn dispatch_frame_changed(frame: u32) {
        onFrameChanged(frame);
    }

    pub fn dispatch_debug_message(message: &str) {
        onDebugMessage(&&safe_string(message));
    }

    pub fn dispatch_debug_content(content: js_sys::Object) {
        onDebugContent(content);
    }

    pub fn dispatch_debug_bitmap(width: u32, height: u32, data: &[u8]) {
        let map = js_sys::Map::new();
        map.str_set("type", &safe_js_string("bitmap"));
        map.str_set("width", &JsValue::from_f64(width as f64));
        map.str_set("height", &JsValue::from_f64(height as f64));
        map.str_set("data", &js_sys::Uint8Array::from(data));
        Self::dispatch_debug_content(map.to_js_object());
    }

    pub fn dispatch_debug_datum(datum_ref: &DatumRef, player: &DirPlayer) {
        let map = js_sys::Map::new();
        map.str_set("type", &safe_js_string("datum"));
        map.str_set("datumRef", &JsValue::from_f64(datum_ref.unwrap() as f64));
        let snapshot = datum_to_js_bridge(datum_ref, player, 0);
        map.str_set("snapshot", &snapshot);
        Self::dispatch_debug_content(map.to_js_object());
    }

    pub fn get_mini_member_snapshot(member: &CastMember) -> js_sys::Map {
        let member_map = js_sys::Map::new();
        member_map.str_set("name", &safe_js_string(&member.name));
        member_map.str_set("type", &safe_js_string(&member.member_type.type_string()));
        if let CastMemberType::Script(script_data) = &member.member_type {
            member_map.str_set("scriptType", &safe_js_string(match script_data.script_type {
                ScriptType::Movie => "movie",
                ScriptType::Parent => "parent",
                ScriptType::Score => "score",
                _ => "unknown",
            }));
        }
        return member_map;
    }

    pub fn get_member_snapshot(
        member: &CastMember,
        cast_lib: u32,
        lctx: Option<&ScriptContext>,
        player: &DirPlayer,
    ) -> js_sys::Map {
        let member_map = js_sys::Map::new();
        member_map.str_set("number", &JsValue::from(member.number));
        member_map.str_set("name", &safe_js_string(&member.name));
        member_map.str_set("type", &safe_js_string(&member.member_type.type_string()));

        match &member.member_type {
            CastMemberType::Field(text_data) => {
                member_map.str_set("text", &ascii_safe(&text_data.text).to_js_value());
            }
            CastMemberType::Text(text_data) => {
                member_map.str_set("text", &ascii_safe(&text_data.text).to_js_value());
                member_map.str_set("htmlSource", &ascii_safe(&text_data.html_source).to_js_value());
                member_map.str_set("alignment", &ascii_safe(&text_data.alignment).to_js_value());
                member_map.str_set("boxType", &ascii_safe(&text_data.box_type).to_js_value());
                member_map.str_set("wordWrap", &JsValue::from_bool(text_data.word_wrap));
                member_map.str_set("antiAlias", &JsValue::from_bool(text_data.anti_alias));
                member_map.str_set("font", &ascii_safe(&text_data.font).to_js_value());
                // set fontStyle array of strings
                let font_style_array = js_sys::Array::new();
                for style in &text_data.font_style {
                    font_style_array.push(&ascii_safe(style).to_js_value());
                }
                member_map.str_set("fontStyle", &font_style_array);
                member_map.str_set("fixedLineSpace", &JsValue::from_f64(text_data.fixed_line_space as f64));
                member_map.str_set("topSpacing", &JsValue::from_f64(text_data.top_spacing as f64));
                member_map.str_set("bottomSpacing", &JsValue::from_f64(text_data.bottom_spacing as f64));
                member_map.str_set("width", &JsValue::from_f64(text_data.width as f64));
                member_map.str_set("height", &JsValue::from_f64(text_data.height as f64));
                // set spans array
                let spans_array = js_sys::Array::new();
                for span in &text_data.html_styled_spans {
                    let span_map = js_sys::Map::new();
                    span_map.str_set("text", &ascii_safe(&span.text).to_js_value());
                    span_map.str_set("fontFace", &ascii_safe(&span.style.font_face.clone().unwrap_or_default()).to_js_value());
                    span_map.str_set("fontSize", &JsValue::from_f64(span.style.font_size.unwrap_or_default() as f64));
                    span_map.str_set("bold", &JsValue::from_bool(span.style.bold));
                    span_map.str_set("italic", &JsValue::from_bool(span.style.italic));
                    span_map.str_set("underline", &JsValue::from_bool(span.style.underline));
                    span_map.str_set("color", &JsValue::from_f64(span.style.color.unwrap_or_default() as f64));
                    spans_array.push(&span_map.to_js_object());
                }
                member_map.str_set("htmlStyledSpans", &spans_array);

            }
            CastMemberType::Script(script_data) => {
                let lctx = lctx.unwrap();
                let script = &lctx.scripts[&script_data.script_id];

                // Get cast info for variable multiplier
                let cast = player
                    .movie
                    .cast_manager
                    .get_cast(cast_lib)
                    .unwrap();
                let capital_x = cast.capital_x;
                let dir_version = cast.dir_version;

                member_map.str_set(
                    "script",
                    &Self::get_script_snapshot(&script_data, &script, &lctx, capital_x, dir_version).to_js_object(),
                );
            }
            CastMemberType::Bitmap(bitmap_data) => {
                let bitmap = player
                    .bitmap_manager
                    .get_bitmap(bitmap_data.image_ref)
                    .unwrap();
                member_map.str_set("width", &JsValue::from(bitmap.width));
                member_map.str_set("height", &JsValue::from(bitmap.height));
                member_map.str_set("bitDepth", &JsValue::from(bitmap.bit_depth));
                member_map.str_set("paletteRef", &bitmap.palette_ref.to_js_value());
                member_map.str_set("regX", &JsValue::from(bitmap_data.reg_point.0));
                member_map.str_set("regY", &JsValue::from(bitmap_data.reg_point.1));
            }
            CastMemberType::FilmLoop(film_loop_data) => {
                member_map.str_set("width", &JsValue::from(film_loop_data.info.width));
                member_map.str_set("height", &JsValue::from(film_loop_data.info.height));
                member_map.str_set("center", &JsValue::from(film_loop_data.info.center));
                member_map.str_set("regX", &JsValue::from(film_loop_data.info.reg_point.0));
                member_map.str_set("regY", &JsValue::from(film_loop_data.info.reg_point.1));
                let score_snapshot = Self::get_score_snapshot(player, &film_loop_data.score);
                member_map.str_set("score", &score_snapshot.to_js_object());
            }
            CastMemberType::Flash(flash_data) => {
                member_map.str_set("regX", &JsValue::from(flash_data.reg_point.0));
                member_map.str_set("regY", &JsValue::from(flash_data.reg_point.1));
                member_map.str_set("dataSize", &JsValue::from(flash_data.data.len() as u32));
                if let Some(ref info) = flash_data.flash_info {
                    member_map.str_set("flashRectLeft", &JsValue::from(info.flash_rect.0));
                    member_map.str_set("flashRectTop", &JsValue::from(info.flash_rect.1));
                    member_map.str_set("flashRectRight", &JsValue::from(info.flash_rect.2));
                    member_map.str_set("flashRectBottom", &JsValue::from(info.flash_rect.3));
                    member_map.str_set("width", &JsValue::from(info.flash_rect.2 - info.flash_rect.0));
                    member_map.str_set("height", &JsValue::from(info.flash_rect.3 - info.flash_rect.1));
                    member_map.str_set("directToStage", &JsValue::from_bool(info.direct_to_stage));
                    member_map.str_set("imageEnabled", &JsValue::from_bool(info.image_enabled));
                    member_map.str_set("soundEnabled", &JsValue::from_bool(info.sound_enabled));
                    member_map.str_set("pausedAtStart", &JsValue::from_bool(info.paused_at_start));
                    member_map.str_set("loop", &JsValue::from_bool(info.loop_enabled));
                    member_map.str_set("isStatic", &JsValue::from_bool(info.is_static));
                    member_map.str_set("preload", &JsValue::from_bool(info.preload));
                    member_map.str_set("centerRegPoint", &JsValue::from_bool(info.center_reg_point));
                    member_map.str_set("buttonsEnabled", &JsValue::from_bool(info.buttons_enabled));
                    member_map.str_set("actionsEnabled", &JsValue::from_bool(info.actions_enabled));
                    member_map.str_set("fixedRate", &JsValue::from(info.fixed_rate));
                    member_map.str_set("posterFrame", &JsValue::from(info.poster_frame));
                    member_map.str_set("bufferSize", &JsValue::from(info.buffer_size));
                    member_map.str_set("scale", &JsValue::from_f64(info.scale as f64));
                    member_map.str_set("viewScale", &JsValue::from_f64(info.view_scale as f64));
                    member_map.str_set("originH", &JsValue::from_f64(info.origin_h as f64));
                    member_map.str_set("originV", &JsValue::from_f64(info.origin_v as f64));
                    member_map.str_set("viewH", &JsValue::from_f64(info.view_h as f64));
                    member_map.str_set("viewV", &JsValue::from_f64(info.view_v as f64));
                    member_map.str_set("originMode", &safe_js_string(match info.origin_mode {
                        crate::director::enums::FlashOriginMode::Center => "center",
                        crate::director::enums::FlashOriginMode::TopLeft => "topLeft",
                        crate::director::enums::FlashOriginMode::Point => "point",
                    }));
                    member_map.str_set("playbackMode", &safe_js_string(match info.playback_mode {
                        crate::director::enums::FlashPlaybackMode::Normal => "normal",
                        crate::director::enums::FlashPlaybackMode::Fixed => "fixed",
                        crate::director::enums::FlashPlaybackMode::LockStep => "lockStep",
                    }));
                    member_map.str_set("scaleMode", &safe_js_string(match info.scale_mode {
                        crate::director::enums::FlashScaleMode::ShowAll => "showAll",
                        crate::director::enums::FlashScaleMode::NoScale => "noScale",
                        crate::director::enums::FlashScaleMode::AutoSize => "autoSize",
                        crate::director::enums::FlashScaleMode::ExactFit => "exactFit",
                        crate::director::enums::FlashScaleMode::NoBorder => "noBorder",
                    }));
                    member_map.str_set("streamMode", &safe_js_string(match info.stream_mode {
                        crate::director::enums::FlashStreamMode::Frame => "frame",
                        crate::director::enums::FlashStreamMode::Idle => "idle",
                        crate::director::enums::FlashStreamMode::Manual => "manual",
                    }));
                    member_map.str_set("quality", &safe_js_string(match info.quality {
                        crate::director::enums::FlashQuality::AutoHigh => "autoHigh",
                        crate::director::enums::FlashQuality::AutoMedium => "autoMedium",
                        crate::director::enums::FlashQuality::AutoLow => "autoLow",
                        crate::director::enums::FlashQuality::High => "high",
                        crate::director::enums::FlashQuality::Medium => "medium",
                        crate::director::enums::FlashQuality::Low => "low",
                    }));
                    member_map.str_set("eventPassMode", &safe_js_string(match info.event_pass_mode {
                        crate::director::enums::FlashEventPassMode::PassAlways => "passAlways",
                        crate::director::enums::FlashEventPassMode::PassButton => "passButton",
                        crate::director::enums::FlashEventPassMode::PassNotButton => "passNotButton",
                        crate::director::enums::FlashEventPassMode::PassNever => "passNever",
                    }));
                    member_map.str_set("clickMode", &safe_js_string(match info.click_mode {
                        crate::director::enums::FlashClickMode::BoundingBox => "boundingBox",
                        crate::director::enums::FlashClickMode::Opaque => "opaque",
                        crate::director::enums::FlashClickMode::Object => "object",
                    }));
                    member_map.str_set("sourceFileName", &safe_js_string(&info.source_file_name));
                    member_map.str_set("commonPlayer", &safe_js_string(&info.common_player));
                    member_map.str_set("bgColor", &JsValue::from(info.bg_color));
                }
            }
            CastMemberType::Palette(palette) => {
                let colors_array = js_sys::Array::new();
                for color in palette.colors.iter() {
                    let color_array = js_sys::Array::new();
                    color_array.push(&JsValue::from_f64(color.0 as f64));
                    color_array.push(&JsValue::from_f64(color.1 as f64));
                    color_array.push(&JsValue::from_f64(color.2 as f64));
                    colors_array.push(&color_array);
                }
                member_map.str_set("colors", &colors_array);
            }
            CastMemberType::Shockwave3d(s3d_data) => {
                let info = &s3d_data.info;
                member_map.str_set("regX", &JsValue::from(info.reg_point.0));
                member_map.str_set("regY", &JsValue::from(info.reg_point.1));
                member_map.str_set("dataSize", &JsValue::from(s3d_data.w3d_data.len() as u32));
                member_map.str_set("directToStage", &JsValue::from_bool(info.direct_to_stage));
                member_map.str_set("animationEnabled", &JsValue::from_bool(info.animation_enabled));
                member_map.str_set("preload", &JsValue::from_bool(info.preload));
                member_map.str_set("loop", &JsValue::from_bool(info.loops));
                member_map.str_set("duration", &JsValue::from(info.duration));
                let rect = info.default_rect;
                member_map.str_set("width", &JsValue::from(rect.2 - rect.0));
                member_map.str_set("height", &JsValue::from(rect.3 - rect.1));
                member_map.str_set("rectLeft", &JsValue::from(rect.0));
                member_map.str_set("rectTop", &JsValue::from(rect.1));
                member_map.str_set("rectRight", &JsValue::from(rect.2));
                member_map.str_set("rectBottom", &JsValue::from(rect.3));
                if let Some(pos) = info.camera_position {
                    let arr = js_sys::Array::new();
                    arr.push(&JsValue::from_f64(pos.0 as f64));
                    arr.push(&JsValue::from_f64(pos.1 as f64));
                    arr.push(&JsValue::from_f64(pos.2 as f64));
                    member_map.str_set("cameraPosition", &arr);
                }
                if let Some(rot) = info.camera_rotation {
                    let arr = js_sys::Array::new();
                    arr.push(&JsValue::from_f64(rot.0 as f64));
                    arr.push(&JsValue::from_f64(rot.1 as f64));
                    arr.push(&JsValue::from_f64(rot.2 as f64));
                    member_map.str_set("cameraRotation", &arr);
                }
                if let Some(bg) = info.bg_color {
                    member_map.str_set("bgColor", &safe_js_string(&format!("rgb({},{},{})", bg.0, bg.1, bg.2)));
                }
                if let Some(ambient) = info.ambient_color {
                    member_map.str_set("ambientColor", &safe_js_string(&format!("rgb({},{},{})", ambient.0, ambient.1, ambient.2)));
                }
                member_map.str_set("hasScene", &JsValue::from_bool(s3d_data.parsed_scene.is_some()));
            }
            _ => {}
        };

        return member_map;
    }

    pub fn get_score_snapshot(player: &DirPlayer, score: &Score) -> js_sys::Map {
        let member_map = js_sys::Map::new();
        member_map.str_set("channelCount", &JsValue::from(score.get_channel_count()));

        member_map.str_set(
            "behaviorReferences",
            &js_sys::Array::from_iter(
                score
                    .sprite_spans
                    .iter()
                    .filter(|span| span.scripts.len() > 0)
                    .map(|span| {
                        let behavior = span.scripts.first().unwrap();
                        let script_ref_map = js_sys::Map::new();
                        script_ref_map.str_set("startFrame", &span.start_frame.to_js_value());
                        script_ref_map.str_set("endFrame", &span.end_frame.to_js_value());
                        script_ref_map.str_set("castLib", &behavior.cast_lib.to_js_value());
                        script_ref_map.str_set("castMember", &behavior.cast_member.to_js_value());
                        script_ref_map.str_set("channelNumber", &span.channel_number.to_js_value());
                        JsValue::from(script_ref_map.to_js_object())
                    }),
            ),
        );

        // Build sprite spans from the raw channel data
        let sprite_spans = Self::create_sprite_spans_from_channels(score, player);
        
        member_map.str_set(
            "spriteSpans",
            &js_sys::Array::from_iter(sprite_spans.iter().map(|span| span.to_js_value())),
        );
        
        member_map.str_set(
            "channelInitData",
            &js_sys::Array::from_iter(score.channel_initialization_data.iter().map(
                |(frame_index, channel_index, init_data)| {
                    let channel_map = js_sys::Map::new();
                    channel_map.str_set("frameIndex", &frame_index.to_js_value());
                    channel_map.str_set("channelIndex", &channel_index.to_js_value());
                    channel_map.str_set(
                        "channelNumber",
                        &get_channel_number_from_index(*channel_index as u32).to_js_value(),
                    );

                    let init_data_map = js_sys::Map::new();
                    init_data_map.str_set("spriteType", &init_data.sprite_type.to_js_value());
                    init_data_map.str_set("castLib", &init_data.cast_lib.to_js_value());
                    init_data_map.str_set("castMember", &init_data.cast_member.to_js_value());
                    init_data_map.str_set("width", &init_data.width.to_js_value());
                    init_data_map.str_set("height", &init_data.height.to_js_value());
                    init_data_map.str_set("locH", &init_data.pos_x.to_js_value());
                    init_data_map.str_set("locV", &init_data.pos_y.to_js_value());
                    init_data_map.str_set("spriteListIdx", &init_data.sprite_list_idx().to_js_value());

                    channel_map.str_set("initData", &init_data_map.to_js_object());
                    JsValue::from(channel_map.to_js_object())
                },
            )),
        );

        return member_map;
    }

    // Create sprite spans by examining actual channel state across frames
    fn create_sprite_spans_from_channels(score: &Score, _player: &DirPlayer) -> Vec<ScoreSpriteSpan> {
        use std::collections::HashMap;
        
        let mut spans = Vec::new();
        let mut channel_data: HashMap<u16, Vec<(u32, u16, u16)>> = HashMap::new();
        
        // Collect all frame data per channel from channel_initialization_data
        for (frame_index, channel_index, init_data) in &score.channel_initialization_data {
            let channel_num = get_channel_number_from_index(*channel_index as u32) as u16;
            let cast_lib = init_data.cast_lib;
            let cast_member = init_data.cast_member;
            
            // Skip empty sprites
            if cast_lib == 0 && cast_member == 0 {
                continue;
            }
            
            channel_data
                .entry(channel_num)
                .or_insert_with(Vec::new)
                .push((*frame_index, cast_lib, cast_member));
        }
        
        // For each channel, create spans from consecutive frames
        for (channel_num, mut frames) in channel_data {
            // Sort by frame
            frames.sort_by_key(|(frame, _, _)| *frame);
            
            let mut current_span: Option<ScoreSpriteSpan> = None;
            
            for (frame, cast_lib, cast_member) in frames {
                let member_ref = [cast_lib, cast_member];
                
                if let Some(ref mut span) = current_span {
                    // Check if this continues the current span
                    if span.member_ref == member_ref && span.end_frame + 1 == frame {
                        // Extend the current span
                        span.end_frame = frame;
                    } else {
                        // Save the current span and start a new one
                        spans.push(span.clone());
                        current_span = Some(ScoreSpriteSpan {
                            channel_number: channel_num,
                            start_frame: frame,
                            end_frame: frame,
                            member_ref,
                        });
                    }
                } else {
                    // Start the first span for this channel
                    current_span = Some(ScoreSpriteSpan {
                        channel_number: channel_num,
                        start_frame: frame,
                        end_frame: frame,
                        member_ref,
                    });
                }
            }
            
            // Don't forget the last span!
            if let Some(span) = current_span {
                spans.push(span);
            }
        }
        
        // Sort spans by channel, then start frame
        spans.sort_by_key(|s| (s.channel_number, s.start_frame));
        
        spans
    }

    pub fn dispatch_channel_name_changed(channel: i16) {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };

            if player.is_subscribed_to_channel_names {
                let display_name =
                    Self::get_channel_display_name(&channel, player).unwrap_or("".to_owned());
                onChannelDisplayNameChanged(channel, &display_name);
            }
        });
    }

    fn get_channel_display_name(channel: &i16, player: &DirPlayer) -> Option<String> {
        let channel = player.movie.score.get_channel(*channel);
        let member_ref = &channel.sprite.member.as_ref();
        if member_ref.is_none() || !member_ref.unwrap().is_valid() {
            return None;
        }
        let member_ref = member_ref.unwrap();
        let member = player.movie.cast_manager.find_member_by_ref(&member_ref);
        if member.is_none() {
            return None;
        }
        let member = member.unwrap();

        // Use safe_string to handle non-ASCII characters (e.g., Japanese)
        if !channel.name.is_empty() {
            return Some(safe_string(&channel.name));
        } else if !channel.sprite.name.is_empty() {
            return Some(safe_string(&channel.sprite.name));
        } else if !member.name.is_empty() {
            return Some(safe_string(&member.name));
        } else {
            return None;
        }
    }

    pub fn get_channel_snapshot(player: &DirPlayer, channel_num: &i16) -> js_sys::Map {
        let channel = player.movie.score.get_channel(*channel_num);
        let result = js_sys::Map::new();

        let member_ref = &channel.sprite.member.as_ref();
        if member_ref.is_none() || !member_ref.unwrap().is_valid() {
            debug!(
                "get_channel_snapshot: ch{} member_ref is None or invalid, puppet={}",
                channel_num, channel.sprite.puppet
            );
            return result;
        }
        let member_ref = member_ref.unwrap();
        let display_name =
            Self::get_channel_display_name(channel_num, player).unwrap_or("".to_owned());

        let member_ref_array = js_sys::Array::new();
        member_ref_array.push(&JsValue::from_f64(member_ref.cast_lib as f64));
        member_ref_array.push(&JsValue::from_f64(member_ref.cast_member as f64));

        let script_instance_array = js_sys::Array::new();
        for script_instance in &channel.sprite.script_instance_list {
            script_instance_array.push(&JsValue::from_f64(**script_instance as f64));
        }

        let sprite_map = js_sys::Map::new();
        sprite_map.str_set("displayName", &display_name.to_js_value());
        sprite_map.str_set("memberRef", &member_ref_array);
        sprite_map.str_set("scriptInstanceList", &script_instance_array);
        sprite_map.str_set("width", &JsValue::from_f64(channel.sprite.width as f64));
        sprite_map.str_set("height", &JsValue::from_f64(channel.sprite.height as f64));
        sprite_map.str_set("locH", &JsValue::from_f64(channel.sprite.loc_h as f64));
        sprite_map.str_set("locV", &JsValue::from_f64(channel.sprite.loc_v as f64));
        sprite_map.str_set("color", &channel.sprite.color.to_string().to_js_value());
        sprite_map.str_set(
            "bgColor",
            &channel.sprite.bg_color.to_string().to_js_value(),
        );
        sprite_map.str_set("ink", &JsValue::from_f64(channel.sprite.ink as f64));
        sprite_map.str_set("blend", &JsValue::from_f64(channel.sprite.blend as f64));

        return sprite_map;
    }

    /// Emit one synthetic "handler" entry per JS function in `ir`. The
    /// top-level program body is also emitted (under the name "(toplevel)")
    /// because it carries the script's var initializers and any
    /// non-function top-level statements — important for diagnosing
    /// missing-constant bugs (bi_bpe, bi_mask, etc.).
    fn push_js_handlers(
        ir: &crate::player::js_lingo::JsScriptIR,
        path_prefix: &str,
        out: &js_sys::Array,
    ) {
        use crate::player::js_lingo::xdr::JsAtom;

        if path_prefix.is_empty() {
            Self::push_one_js_handler(ir, "(toplevel)", &[], out);
        }

        for atom in &ir.atoms {
            if let JsAtom::Function(fa) = atom {
                let handler_name = match (path_prefix.is_empty(), &fa.name) {
                    (true, Some(n)) => n.clone(),
                    (true, None) => "(anonymous)".to_string(),
                    (false, Some(n)) => format!("{}.{}", path_prefix, n),
                    (false, None) => format!("{}.(anonymous)", path_prefix),
                };
                let arg_names: Vec<String> = fa.bindings.iter()
                    .filter(|b| b.kind == crate::player::js_lingo::xdr::JsBindingKind::Argument)
                    .map(|b| b.name.clone())
                    .collect();
                Self::push_one_js_handler_with_bindings(&fa.script, &handler_name, &arg_names, &fa.bindings, out);
                // Only drill into nested closures if this function actually
                // declares some (i.e. its atom map contains JsAtom::Function).
                let has_nested = fa.script.atoms.iter().any(|a| matches!(a, JsAtom::Function(_)));
                if has_nested {
                    Self::push_js_handlers(&fa.script, &handler_name, out);
                }
            }
        }
    }

    fn push_one_js_handler(
        ir: &crate::player::js_lingo::JsScriptIR,
        name: &str,
        arg_names: &[String],
        out: &js_sys::Array,
    ) {
        Self::push_one_js_handler_with_bindings(ir, name, arg_names, &[], out);
    }

    fn push_one_js_handler_with_bindings(
        ir: &crate::player::js_lingo::JsScriptIR,
        name: &str,
        arg_names: &[String],
        bindings: &[crate::player::js_lingo::xdr::JsFunctionBinding],
        out: &js_sys::Array,
    ) {
        use crate::player::js_lingo::opcodes::JsOpFormat;
        use crate::player::js_lingo::variable_length::{read_i16_operand, read_u16_operand};
        use crate::player::js_lingo::xdr::iter_ops;

        let handler_map = js_sys::Map::new();
        handler_map.str_set("name", &name.to_owned().to_js_value());

        let args_array = js_sys::Array::new();
        for a in arg_names { args_array.push(&a.clone().to_js_value()); }
        handler_map.str_set("args", &args_array);

        let bytecode_array = js_sys::Array::new();
        let lingo_array = js_sys::Array::new();
        let mut bc_to_line: Vec<(usize, usize)> = Vec::new();

        // Decompile to JS source — this is what the "Lingo" tab displays.
        let decomp = crate::player::js_lingo::decompiler::decompile(ir, bindings);
        let decomp_bc_to_line: std::collections::HashMap<usize, usize> =
            decomp.bytecode_to_line.iter().copied().collect();
        for line in &decomp.lines {
            let line_map = js_sys::Map::new();
            line_map.str_set("text", &line.text.clone().to_js_value());
            line_map.str_set("indent", &JsValue::from(line.indent));
            let idx_arr = js_sys::Array::new();
            for bc in &line.bytecode_indices {
                idx_arr.push(&JsValue::from(*bc as u32));
            }
            line_map.str_set("bytecodeIndices", &idx_arr);
            line_map.str_set("spans", &js_sys::Array::new());
            lingo_array.push(&line_map.to_js_object());
        }

        for (i, ins) in iter_ops(&ir.bytecode).enumerate() {
            let ins = match ins {
                Ok(i) => i,
                Err(e) => {
                    let map = js_sys::Map::new();
                    map.str_set("pos", &JsValue::from(0u32));
                    map.str_set("text", &format!("<decode error: {}>", e).to_js_value());
                    bytecode_array.push(&map.to_js_object());
                    continue;
                }
            };
            let info = ins.op.info();
            let operand_str = match info.format {
                JsOpFormat::Byte => String::new(),
                JsOpFormat::Uint16 | JsOpFormat::Qarg | JsOpFormat::Qvar | JsOpFormat::Local => {
                    read_u16_operand(ins.operand).map(|v| format!(" {}", v)).unwrap_or_default()
                }
                JsOpFormat::Const => {
                    if let Ok(idx) = read_u16_operand(ins.operand) {
                        let lbl = ir.atoms.get(idx as usize).map(format_atom_summary).unwrap_or_else(|| "<oob>".into());
                        format!(" #{} ; {}", idx, lbl)
                    } else { String::new() }
                }
                JsOpFormat::Jump => {
                    read_i16_operand(ins.operand).map(|d| format!(" {:+} ; -> {}", d, ins.offset as i32 + d as i32)).unwrap_or_default()
                }
                JsOpFormat::Object => {
                    read_u16_operand(ins.operand).map(|v| format!(" obj#{}", v)).unwrap_or_default()
                }
                JsOpFormat::Tableswitch | JsOpFormat::Lookupswitch => format!(" <{} bytes>", ins.operand.len()),
            };
            let text = format!("{:>4}: {:<14}{}", ins.offset, info.mnemonic, operand_str);
            let map = js_sys::Map::new();
            map.str_set("pos", &JsValue::from(ins.offset as u32));
            map.str_set("text", &text.to_js_value());
            bytecode_array.push(&map.to_js_object());

            // bc → line mapping: use the decompiler's mapping. Any bytecode
            // not covered (e.g. structural markers like POP after Pushobj)
            // points at the nearest source line if we have one, else line 0.
            if let Some(line_idx) = decomp_bc_to_line.get(&i) {
                bc_to_line.push((i, *line_idx));
            }
        }
        let _ = bc_to_line; // populated for completeness; the map below uses decomp_bc_to_line directly
        handler_map.str_set("bytecode", &bytecode_array);
        handler_map.str_set("lingo", &lingo_array);
        let mapping_obj = js_sys::Object::new();
        for (bc, ln) in &decomp_bc_to_line {
            js_sys::Reflect::set(&mapping_obj, &JsValue::from(*bc as u32), &JsValue::from(*ln as u32)).ok();
        }
        handler_map.str_set("bytecodeToLine", &mapping_obj);
        out.push(&handler_map.to_js_object());
    }

    pub fn get_script_snapshot(
        member: &ScriptMember,
        chunk: &ScriptChunk,
        lctx: &ScriptContext,
        capital_x: bool,
        dir_version: u16,
    ) -> js_sys::Map {
        let member_map = js_sys::Map::new();
        member_map.str_set("name", &member.name.to_js_value());
        member_map.str_set(
            "script_type",
            &match member.script_type {
                ScriptType::Movie => "movie".to_owned().to_js_value(),
                ScriptType::Parent => "parent".to_owned().to_js_value(),
                ScriptType::Score => "score".to_owned().to_js_value(),
                _ => "unknown".to_owned().to_js_value(),
            },
        );

        // JS-Lingo path: the Lscr's literal-data region holds an XDR-serialized
        // SpiderMonkey script. We replace the handler array with a synthetic
        // one whose "bytecode" view is the JS disassembly and whose "lingo"
        // view is a placeholder comment header. Lingo handlers in this
        // chunk (if any) get appended afterwards.
        if let Some(js_payload) = chunk.literals.iter().find_map(|l| match l {
            crate::director::lingo::datum::Datum::JavaScript(b) => Some(b.as_slice()),
            _ => None,
        }) {
            member_map.str_set("script_syntax", &"javascript".to_owned().to_js_value());
            if let Ok(ir) = crate::player::js_lingo::decode_script(js_payload) {
                let handlers_array = js_sys::Array::new();
                Self::push_js_handlers(&ir, "", &handlers_array);
                member_map.str_set("handlers", &handlers_array);
                return member_map;
            }
        }

        member_map.str_set("script_syntax", &"lingo".to_owned().to_js_value());

        // Calculate multiplier once
        let multiplier = get_variable_multiplier(capital_x, dir_version);

        let handlers_array = js_sys::Array::new();
        for handler in &chunk.handlers {
            let handler_map = js_sys::Map::new();
            let bytecode_array = js_sys::Array::new();
            let args_array = js_sys::Array::new();
            let name = &lctx.names[handler.name_id as usize];

            for bytecode in &handler.bytecode_array {
                let bytecode_map = js_sys::Map::new();

                bytecode_map.str_set("pos", &JsValue::from(bytecode.pos));
                bytecode_map.str_set(
                    "text",
                    &bytecode.to_bytecode_text(lctx, &handler, multiplier).to_js_value(),
                );

                bytecode_array.push(&bytecode_map.to_js_object());
            }

            for arg in &handler.argument_name_ids {
                args_array.push(&lctx.names[*arg as usize].to_js_value());
            }

            handler_map.str_set("name", &name.to_js_value());
            handler_map.str_set("args", &args_array);
            handler_map.str_set("bytecode", &bytecode_array);

            // Decompile handler to Lingo source
            let decompiled = decompiler::decompile_handler(handler, chunk, lctx, dir_version, multiplier);

            // Add lingo lines
            let lingo_array = js_sys::Array::new();
            for line in &decompiled.lines {
                let line_map = js_sys::Map::new();
                line_map.str_set("text", &line.text.to_js_value());
                line_map.str_set("indent", &JsValue::from(line.indent));

                let indices_array = js_sys::Array::new();
                for &idx in &line.bytecode_indices {
                    indices_array.push(&JsValue::from(idx as u32));
                }
                line_map.str_set("bytecodeIndices", &indices_array);

                // Add syntax highlighting spans
                let spans_array = js_sys::Array::new();
                for span in &line.spans {
                    let span_map = js_sys::Map::new();
                    span_map.str_set("text", &span.text.to_js_value());
                    span_map.str_set("type", &JsValue::from_str(span.token_type.as_str()));
                    spans_array.push(&span_map.to_js_object());
                }
                line_map.str_set("spans", &spans_array);

                lingo_array.push(&line_map.to_js_object());
            }
            handler_map.str_set("lingo", &lingo_array);

            // Add bytecode to line mapping
            let mapping_obj = js_sys::Object::new();
            for (&bc_idx, &line_idx) in &decompiled.bytecode_to_line {
                js_sys::Reflect::set(
                    &mapping_obj,
                    &JsValue::from(bc_idx as u32),
                    &JsValue::from(line_idx as u32),
                ).ok();
            }
            handler_map.str_set("bytecodeToLine", &mapping_obj);

            handlers_array.push(&handler_map.to_js_object());
        }
        member_map.str_set("handlers", &handlers_array);

        return member_map;
    }

    pub fn dispatch_scope_list(player: &DirPlayer) {
        onScopeListChanged(
            player
                .scopes
                .iter()
                .enumerate()
                .filter(|(i, _)| player.scope_count > *i as u32)
                .map(|(_, scope)| {
                    let cast_lib = player
                        .movie
                        .cast_manager
                        .get_cast(scope.script_ref.cast_lib as u32)
                        .unwrap();
                    let handler_name = cast_lib
                        .lctx
                        .as_ref()
                        .unwrap()
                        .names
                        .get(scope.handler_name_id as usize)
                        .unwrap();
                    let names = &cast_lib.lctx.as_ref().unwrap().names;
                    let scope = JsBridgeScope {
                        script_member_ref: scope.script_ref.to_js(),
                        bytecode_index: scope.bytecode_index as u32,
                        handler_name: handler_name.to_owned(),
                        locals: scope
                            .locals
                            .iter()
                            .map(|(name_id, v)| {
                                let name = names.get(*name_id as usize)
                                    .cloned()
                                    .unwrap_or_else(|| format!("local_{}", name_id));
                                (name, v.clone())
                            })
                            .collect(),
                        stack: scope.stack.clone(),
                        args: scope.args.clone(),
                    };
                    let scope_js: js_sys::Map = scope.into();
                    scope_js.to_js_object()
                })
                .collect(),
        );
    }

    pub fn dispatch_global_list(player: &DirPlayer) {
        let globals = js_sys::Map::new();
        for (k, v) in player.globals.iter() {
            globals.set(&safe_js_string(&k.to_string()), &v.unwrap().to_js_value());
        }
        onGlobalListChanged(globals.to_js_object());
    }

    pub fn dispatch_debug_update(player: &DirPlayer) {
        Self::dispatch_scope_list(player);
        Self::dispatch_global_list(player);
    }

    pub fn dispatch_script_error(player: &DirPlayer, err: &ScriptError) {
        let is_paused = player.current_breakpoint.as_ref()
            .map(|bp| bp.error.is_some())
            .unwrap_or(false);

        let data: js_sys::Map =
            if let Some(current_scope) = player.scopes.get(player.current_scope_ref()) {
                let cast_lib = player
                    .movie
                    .cast_manager
                    .get_cast(current_scope.script_ref.cast_lib as u32)
                    .unwrap();
                let current_handler_name = cast_lib
                    .lctx
                    .as_ref()
                    .unwrap()
                    .names
                    .get(current_scope.handler_name_id as usize)
                    .unwrap();

                OnScriptErrorCallbackData {
                    message: err.message.to_owned(),
                    script_member_ref: Some(current_scope.script_ref.to_js()),
                    handler_name: Some(current_handler_name.to_owned()),
                    is_paused,
                }
                .into()
            } else {
                OnScriptErrorCallbackData {
                    message: err.message.to_owned(),
                    script_member_ref: None,
                    handler_name: None,
                    is_paused,
                }
                .into()
            };

        onScriptError(data.to_js_object());
    }

    pub fn dispatch_breakpoint_list_changed() {
        async_std::task::spawn_local(async move {
            let player = unsafe { PLAYER_OPT.as_ref().unwrap() };
            let breakpoints = player
                .breakpoint_manager
                .breakpoints
                .iter()
                .map(|x| {
                    let breakpoint = JsBridgeBreakpoint {
                        script_name: x.script_name.to_owned(),
                        handler_name: x.handler_name.to_owned(),
                        bytecode_index: x.bytecode_index,
                    };
                    let breakpoint_js: js_sys::Map = breakpoint.into();
                    breakpoint_js.to_js_object()
                })
                .collect();
            onBreakpointListChanged(breakpoints);
        });
    }

    pub fn dispatch_script_error_cleared() {
        onScriptErrorCleared();
    }

    pub fn dispatch_external_event(event: &str) {
        onExternalEvent(event);
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl JsApi {
    pub fn dispatch_datum_snapshot(_: &DatumRef, _: &DirPlayer) {}
    pub fn dispatch_script_instance_snapshot(_: Option<ScriptInstanceRef>, _: &DirPlayer) {}
    pub fn dispatch_schedule_timeout(_: &str, _: u32) {}
    pub fn dispatch_clear_timeout(_: &str) {}
    #[allow(dead_code)]
    pub fn dispatch_clear_timeouts() {}
    pub fn dispatch_movie_loaded(_: &DirectorFile) {}
    pub fn dispatch_movie_load_failed(_: &str, _: &str) {}
    pub fn dispatch_flash_member_loaded(_: i32, _: i32, _: &[u8], _: u32, _: u32) {}
    pub fn dispatch_flash_member_unloaded(_: i32, _: i32) {}
    pub fn dispatch_stage_size_changed(_: u32, _: u32, _: bool) {}
    pub fn dispatch_cast_name_changed(_: u32) {}
    pub fn dispatch_cast_list_changed() {}
    pub fn dispatch_cast_member_list_changed(_: u32) {}
    pub fn dispatch_cast_member_changed(_: CastMemberRef) {}
    pub fn on_cast_member_name_changed(_: u32) {}
    pub fn on_sprite_member_changed(_: i16) {}
    pub fn dispatch_score_changed() {}
    pub fn dispatch_channel_changed(_: i16) {}
    pub fn dispatch_frame_changed(_: u32) {}
    pub fn dispatch_debug_message(_: &str) {}
    pub fn dispatch_debug_content(_: js_sys::Object) {}
    pub fn dispatch_debug_bitmap(_: u32, _: u32, _: &[u8]) {}
    pub fn dispatch_debug_datum(_: &DatumRef, _: &DirPlayer) {}
    pub fn dispatch_channel_name_changed(_: i16) {}
    pub fn dispatch_scope_list(_: &DirPlayer) {}
    pub fn dispatch_global_list(_: &DirPlayer) {}
    pub fn dispatch_debug_update(_: &DirPlayer) {}
    pub fn dispatch_script_error(_: &DirPlayer, _: &ScriptError) {}
    pub fn dispatch_breakpoint_list_changed() {}
    pub fn dispatch_script_error_cleared() {}
    pub fn dispatch_external_event(_: &str) {}
    pub fn get_cast_chunk_list_for(_: &DirPlayer, _: u32) -> js_sys::Object { unimplemented!() }
    pub fn get_movie_top_level_chunks(_: &DirPlayer) -> js_sys::Object { unimplemented!() }
    pub fn get_chunk_bytes(_: &DirPlayer, _: u32, _: u32) -> Option<Vec<u8>> { unimplemented!() }
    pub fn get_parsed_chunk(_: &DirPlayer, _: u32, _: u32) -> js_sys::Object { unimplemented!() }
    pub fn get_mini_member_snapshot(_: &CastMember) -> js_sys::Map { unimplemented!() }
    pub fn get_member_snapshot(_: &CastMember, _: u32, _: Option<&ScriptContext>, _: &DirPlayer) -> js_sys::Map { unimplemented!() }
    pub fn get_score_snapshot(_: &DirPlayer, _: &Score) -> js_sys::Map { unimplemented!() }
    pub fn get_channel_snapshot(_: &DirPlayer, _: &i16) -> js_sys::Map { unimplemented!() }
    fn get_channel_display_name(_: &i16, _: &DirPlayer) -> Option<String> { unimplemented!() }
    pub fn get_script_snapshot(_: &ScriptMember, _: &ScriptChunk, _: &ScriptContext, _: bool, _: u16) -> js_sys::Map { unimplemented!() }
    fn collect_cast_descendants(_: u32, _: &HashMap<u32, Vec<u32>>) -> std::collections::HashSet<u32> { unimplemented!() }
    fn build_children_map(_: &DirectorFile) -> HashMap<u32, Vec<u32>> { unimplemented!() }
}

pub trait JsSerializable {
    fn to_js_object(&self) -> js_sys::Object;
}

pub trait JsUtils {
    fn str_set(&self, key: &str, value: &JsValue);
}

impl JsSerializable for js_sys::Map {
    fn to_js_object(&self) -> js_sys::Object {
        return js_sys::Object::from_entries(self).unwrap();
    }
}

impl JsUtils for js_sys::Map {
    fn str_set(&self, key: &str, value: &JsValue) {
        self.set(&safe_js_string(key), value);
    }
}

fn datum_to_js_bridge(datum_ref: &DatumRef, player: &DirPlayer, depth: u8) -> JsBridgeDatum {
    let datum = player.get_datum(datum_ref);
    concrete_datum_to_js_bridge(datum, player, depth)
}

fn concrete_datum_to_js_bridge(datum: &Datum, player: &DirPlayer, depth: u8) -> JsBridgeDatum {
    if depth > 20 {
        let map = js_sys::Map::new();
        map.str_set("debugDescription", &safe_js_string("TOO DEEP"));
        return map.to_js_object();
    }
    let map = js_sys::Map::new();
    let formatted_value = format_concrete_datum(datum, player);
    map.str_set(
        "debugDescription",
        &ascii_safe(&formatted_value).to_js_value(),
    );
    match datum {
        Datum::String(val) => {
            map.str_set("type", &safe_js_string("string"));
            map.str_set("value", &safe_js_string(&ascii_safe(val)));
        }
        Datum::Int(val) => {
            map.str_set("type", &safe_js_string("number"));
            map.str_set("value", &JsValue::from_f64(*val as f64));
        }
        Datum::Symbol(val) => {
            map.str_set("type", &safe_js_string("symbol"));
            map.str_set("value", &safe_js_string(val));
        }
        Datum::List(_, item_refs, _) => {
            map.str_set("type", &safe_js_string("list"));
            map.str_set(
                "items",
                &item_refs
                    .iter()
                    .map(|x| x.unwrap())
                    .collect_vec()
                    .to_js_value(),
            );
        }
        Datum::VarRef(_) => {
            map.str_set("type", &safe_js_string("var_ref"));
        }
        Datum::Float(val) => {
            map.str_set("type", &safe_js_string("number"));
            map.str_set("numericValue", &JsValue::from_f64(*val as f64));
            map.str_set("value", &safe_js_string(&format_float_with_precision(*val, player)));
        }
        Datum::Void => {
            map.str_set("type", &safe_js_string("void"));
        }
        Datum::CastLib(val) => {
            map.str_set("type", &safe_js_string("castLib"));
            map.str_set("value", &JsValue::from_f64(*val as f64));
        }
        Datum::Stage => {
            map.str_set("type", &safe_js_string("stage"));
        }
        Datum::PropList(properties, sorted) => {
            map.str_set("type", &safe_js_string("propList"));
            let props_map = js_sys::Map::new();
            for (k, v) in properties.iter() {
                let key_str = format_datum(k, player);
                props_map.set(&safe_js_string(&key_str), &v.unwrap().to_js_value());
            }
            map.str_set("properties", &props_map.to_js_object());
            map.str_set("sorted", &JsValue::from_bool(*sorted));
        }
        Datum::StringChunk(..) => {
            map.str_set("type", &safe_js_string("stringChunk"));
        }
        Datum::ScriptRef(_) => {
            map.str_set("type", &safe_js_string("scriptRef"));
        }
        Datum::ScriptInstanceRef(instance_id) => {
            map.str_set("type", &safe_js_string("scriptInstance"));
            let instance = player.allocator.get_script_instance(&instance_id);
            let ancestor_id = &instance.ancestor;
            match ancestor_id {
                Some(ancestor_id) => {
                    map.str_set("ancestor", &(**ancestor_id).to_js_value());
                }
                None => map.str_set("ancestor", &JsValue::NULL),
            }

            let props_map = js_sys::Map::new();
            for (k, v) in instance.properties.iter() {
                props_map.set(&safe_js_string(k.as_str()), &v.unwrap().to_js_value());
            }
            map.str_set("properties", &props_map.to_js_object());
        }
        Datum::CastMember(_) => {
            map.str_set("type", &safe_js_string("castMember"));
        }
        Datum::SpriteRef(_) => {
            map.str_set("type", &safe_js_string("spriteRef"));
        }
        Datum::Rect(vals, flags) => {
            let x1 = Datum::inline_component_to_datum(vals[0], Datum::inline_is_float(*flags, 0));
            let y1 = Datum::inline_component_to_datum(vals[1], Datum::inline_is_float(*flags, 1));
            let x2 = Datum::inline_component_to_datum(vals[2], Datum::inline_is_float(*flags, 2));
            let y2 = Datum::inline_component_to_datum(vals[3], Datum::inline_is_float(*flags, 3));

            map.str_set("type", &safe_js_string("Rect"));
            map.str_set("left", &concrete_datum_to_js_bridge(&x1, player, depth + 1));
            map.str_set("top", &concrete_datum_to_js_bridge(&y1, player, depth + 1));
            map.str_set("right", &concrete_datum_to_js_bridge(&x2, player, depth + 1));
            map.str_set("bottom", &concrete_datum_to_js_bridge(&y2, player, depth + 1));
            map.str_set("value", &safe_js_string(&format!(
                "rect({}, {}, {}, {})",
                if Datum::inline_is_float(*flags, 0) { format!("{:.4}", vals[0]) } else { format!("{}", vals[0] as i32) },
                if Datum::inline_is_float(*flags, 1) { format!("{:.4}", vals[1]) } else { format!("{}", vals[1] as i32) },
                if Datum::inline_is_float(*flags, 2) { format!("{:.4}", vals[2]) } else { format!("{}", vals[2] as i32) },
                if Datum::inline_is_float(*flags, 3) { format!("{:.4}", vals[3]) } else { format!("{}", vals[3] as i32) },
            )));
        }
        Datum::Point(vals, flags) => {
            let x = Datum::inline_component_to_datum(vals[0], Datum::inline_is_float(*flags, 0));
            let y = Datum::inline_component_to_datum(vals[1], Datum::inline_is_float(*flags, 1));

            map.str_set("type", &safe_js_string("Point"));
            map.str_set("x", &concrete_datum_to_js_bridge(&x, player, depth + 1));
            map.str_set("y", &concrete_datum_to_js_bridge(&y, player, depth + 1));
            map.str_set("value", &safe_js_string(&format!(
                "point({}, {})",
                if Datum::inline_is_float(*flags, 0) { format!("{:.4}", vals[0]) } else { format!("{}", vals[0] as i32) },
                if Datum::inline_is_float(*flags, 1) { format!("{:.4}", vals[1]) } else { format!("{}", vals[1] as i32) },
            )));
        }
        Datum::CursorRef(cursor_ref) => {
            map.str_set("type", &safe_js_string("cursorRef"));
            match cursor_ref {
                CursorRef::System(id) => {
                    map.str_set("cursorType", &safe_js_string("system"));
                    map.str_set("id", &JsValue::from(*id));
                }
                CursorRef::Member(member_ref) => {
                    map.str_set("cursorType", &safe_js_string("member"));
                    map.str_set("memberRef", &member_ref.to_js_value());
                }
            }
        }
        Datum::TimeoutRef(name) => {
            map.str_set("type", &safe_js_string("timeout"));
            map.str_set("name", &safe_js_string(name));
        }
        Datum::TimeoutFactory => {
            map.str_set("type", &safe_js_string("timeoutFactory"));
        }
        Datum::TimeoutInstance { name, .. } => {
            map.str_set("type", &safe_js_string("timeoutInstance"));
            map.str_set("name", &safe_js_string(name));
        }
        Datum::ColorRef(color_ref) => {
            map.str_set("type", &safe_js_string("colorRef"));
            match color_ref {
                ColorRef::PaletteIndex(i) => {
                    map.str_set("paletteIndex", &JsValue::from(*i));
                }
                ColorRef::Rgb(r, g, b) => {
                    map.str_set("r", &JsValue::from(*r));
                    map.str_set("g", &JsValue::from(*g));
                    map.str_set("b", &JsValue::from(*b));
                }
            }
        }
        Datum::BitmapRef(bitmap_ref) => {
            map.str_set("type", &safe_js_string("bitmapRef"));
            if let Some(bitmap) = player.bitmap_manager.get_bitmap(*bitmap_ref) {
                map.str_set("width", &JsValue::from(bitmap.width));
                map.str_set("height", &JsValue::from(bitmap.height));
                map.str_set("bitDepth", &JsValue::from(bitmap.bit_depth));
            }
        }
        Datum::PaletteRef(palette_ref) => {
            map.str_set("type", &safe_js_string("paletteRef"));
            map.str_set("value", &palette_ref.to_js_value());
        }
        Datum::Xtra(name) => {
            map.str_set("type", &safe_js_string("xtra"));
            map.str_set("name", &safe_js_string(name));
        }
        Datum::XtraInstance(name, instance_id) => {
            map.str_set("type", &safe_js_string("xtraInstance"));
            map.str_set("name", &safe_js_string(name));
            map.str_set("instanceId", &JsValue::from(*instance_id));
        }
        Datum::Matte(..) => {
            map.str_set("type", &safe_js_string("matte"));
        }
        Datum::Null => {
            map.str_set("type", &safe_js_string("null"));
        }
        Datum::PlayerRef => {
            map.str_set("type", &safe_js_string("playerRef"));
        }
        Datum::MovieRef => {
            map.str_set("type", &safe_js_string("movieRef"));
        }
        Datum::MouseRef => {
            map.str_set("type", &safe_js_string("mouseRef"));
        }
        Datum::SoundRef(sound_id) => {
            map.str_set("type", &safe_js_string("sound"));
            map.str_set("id", &JsValue::from(*sound_id));
        }
        Datum::SoundChannel(channel_id) => {
            map.str_set("type", &safe_js_string("soundChannel"));
            map.str_set("channel", &JsValue::from(*channel_id));
        }
        Datum::XmlRef(id) => {
            map.str_set("type", &safe_js_string("xmlRef"));
            map.str_set("id", &JsValue::from_f64(*id as f64));
        }
        Datum::DateRef(_) => {
            map.str_set("type", &safe_js_string("date"));
        }
        Datum::MathRef(_) => {
            map.str_set("type", &safe_js_string("math"));
        }
        Datum::Vector(vec) => {
            map.str_set("type", &safe_js_string("vector"));
            let vec_array = js_sys::Array::new();
            for val in vec.iter() {
                vec_array.push(&JsValue::from_f64(*val as f64));
            }
            map.str_set("values", &vec_array);
        }
        Datum::Media(_) => {
            map.str_set("type", &safe_js_string("media"));
        }
        Datum::JavaScript(data) => {
            map.str_set("type", &safe_js_string("javascript"));
            map.str_set("size", &JsValue::from(data.len() as f64));
            map.str_set("bytes", &js_sys::Uint8Array::from(&data[..]));
        }
        Datum::FlashObjectRef(flash_ref) => {
            map.str_set("type", &safe_js_string("flashObject"));
            map.str_set("value", &safe_js_string(&flash_ref.path));
        }
        Datum::Shockwave3dObjectRef(s3d_ref) => {
            map.str_set("type", &safe_js_string("shockwave3dObject"));
            map.str_set("value", &safe_js_string(&format!("{}(\"{}\")", s3d_ref.object_type, s3d_ref.name)));
        }
        Datum::Transform3d(_) => {
            map.str_set("type", &safe_js_string("transform"));
        }
        Datum::HavokObjectRef(hk_ref) => {
            map.str_set("type", &safe_js_string("havokObject"));
            map.str_set("value", &safe_js_string(&format!("{}(\"{}\")", hk_ref.object_type, hk_ref.name)));
        }
    }
    return map.to_js_object();
}

pub trait ToJsValue {
    fn to_js_value(&self) -> JsValue;
}

impl ToJsValue for String {
    fn to_js_value(&self) -> JsValue {
        safe_js_string(self)
    }
}

impl ToJsValue for u8 {
    fn to_js_value(&self) -> JsValue {
        JsValue::from_f64(*self as f64)
    }
}

impl ToJsValue for u32 {
    fn to_js_value(&self) -> JsValue {
        JsValue::from_f64(*self as f64)
    }
}

impl ToJsValue for usize {
    fn to_js_value(&self) -> JsValue {
        JsValue::from_f64(*self as f64)
    }
}

impl ToJsValue for u16 {
    fn to_js_value(&self) -> JsValue {
        JsValue::from_f64(*self as f64)
    }
}

impl ToJsValue for i16 {
    fn to_js_value(&self) -> JsValue {
        JsValue::from_f64(*self as f64)
    }
}

impl ToJsValue for PaletteRef {
    fn to_js_value(&self) -> JsValue {
        match self {
            PaletteRef::BuiltIn(id) => safe_js_string(&id.symbol_string()),
            PaletteRef::Member(member_ref) => safe_js_string(
                format!(
                    "(member {} of castLib {})",
                    member_ref.cast_member, member_ref.cast_lib
                )
                .as_str(),
            ),
            PaletteRef::Default => safe_js_string("#default"),
        }
    }
}
