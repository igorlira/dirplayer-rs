use std::{collections::HashMap, iter::FromIterator};

use itertools::Itertools;
use js_sys::{Array, Object};
use wasm_bindgen::prelude::*;

use crate::{
    director::{
        chunks::{script::ScriptChunk, ChunkContainer, score::ScoreFrameChannelData},
        enums::ScriptType,
        file::{DirectorFile, get_variable_multiplier},
        lingo::{datum::Datum, decompiler, script::ScriptContext},
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
        reserve_player_ref,
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

pub fn safe_js_string(s: &str) -> JsValue {
    JsValue::from_str(&safe_string(s))
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
    pub fn onMovieChunkListChanged(chunks: Object);
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
    pub fn onScheduleTimeout(timeout_name: &str, interval: u32);
    pub fn onClearTimeout(timeout_name: &str);
    pub fn onClearTimeouts();
    pub fn onDatumSnapshot(datum_id: DatumId, data: js_sys::Object);
    pub fn onScriptInstanceSnapshot(script_ref: ScriptInstanceId, data: js_sys::Object);
    pub fn onExternalEvent(event: &str);
}

pub struct JsApi {}

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
    pub fn dispatch_movie_loaded(dir_file: &DirectorFile) {
        let test = dir_file
            .cast_entries
            .iter()
            .map(|cast| cast.name.to_owned())
            .collect_vec()
            .join(", ");

        let chunk_list = Self::get_chunk_container_map(&dir_file.chunk_container);
        onMovieLoaded(OnMovieLoadedCallbackData {
            version: dir_file.version,
            test_val: test,
        });
        onMovieChunkListChanged(chunk_list.to_js_object())
    }

    fn get_chunk_container_map(chunk_container: &ChunkContainer) -> js_sys::Map {
        let result = js_sys::Map::new();
        for (chunk_id, chunk) in &chunk_container.chunk_info {
            let fourcc_str = fourcc_to_string(chunk.fourcc);
            let chunk_map = js_sys::Map::new();
            chunk_map.str_set("id", &JsValue::from_f64(*chunk_id as f64));
            chunk_map.str_set("fourcc", &safe_js_string(&fourcc_str));

            result.set(
                &JsValue::from_f64(*chunk_id as f64),
                &chunk_map.to_js_object(),
            );
        }
        return result;
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
            let cast = player.movie.cast_manager.get_cast(cast_number).unwrap();
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
            x.borrow()
                .as_ref()
                .and_then(|y| y.debug_selected_channel_num)
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

    pub fn get_mini_member_snapshot(member: &CastMember) -> js_sys::Map {
        let member_map = js_sys::Map::new();
        member_map.str_set("name", &safe_js_string(&member.name));
        member_map.str_set("type", &safe_js_string(&member.member_type.type_string()));
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
                        script_ref_map.to_js_object()
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
                    channel_map.to_js_object()
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

        if !channel.name.is_empty() {
            return Some(channel.name.clone());
        } else if !channel.sprite.name.is_empty() {
            return Some(channel.sprite.name.clone());
        } else if !member.name.is_empty() {
            return Some(member.name.clone());
        } else {
            return None;
        }
    }

    pub fn get_channel_snapshot(player: &DirPlayer, channel_num: &i16) -> js_sys::Map {
        let channel = player.movie.score.get_channel(*channel_num);
        let result = js_sys::Map::new();

        let member_ref = &channel.sprite.member.as_ref();
        if member_ref.is_none() || !member_ref.unwrap().is_valid() {
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
                    let scope = JsBridgeScope {
                        script_member_ref: scope.script_ref.to_js(),
                        bytecode_index: scope.bytecode_index as u32,
                        handler_name: handler_name.to_owned(),
                        locals: scope
                            .locals
                            .clone()
                            .into_iter()
                            .map(|(k, v)| (k.to_owned(), v))
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
                }
                .into()
            } else {
                OnScriptErrorCallbackData {
                    message: err.message.to_owned(),
                    script_member_ref: None,
                    handler_name: None,
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
                props_map.set(&safe_js_string(k), &v.unwrap().to_js_value());
            }
            map.str_set("properties", &props_map.to_js_object());
        }
        Datum::CastMember(_) => {
            map.str_set("type", &safe_js_string("castMember"));
        }
        Datum::SpriteRef(_) => {
            map.str_set("type", &safe_js_string("spriteRef"));
        }
        Datum::Rect(arr) => {
            let x1 = player.get_datum(&arr[0]);
            let y1 = player.get_datum(&arr[1]);
            let x2 = player.get_datum(&arr[2]);
            let y2 = player.get_datum(&arr[3]);
            
            map.str_set("type", &safe_js_string("Rect"));
            map.str_set("left", &concrete_datum_to_js_bridge(x1, player, depth + 1));
            map.str_set("top", &concrete_datum_to_js_bridge(y1, player, depth + 1));
            map.str_set("right", &concrete_datum_to_js_bridge(x2, player, depth + 1));
            map.str_set("bottom", &concrete_datum_to_js_bridge(y2, player, depth + 1));
            map.str_set("value", &safe_js_string(&format!(
                "rect({}, {}, {}, {})",
                format_numeric_value(x1, player),
                format_numeric_value(y1, player),
                format_numeric_value(x2, player),
                format_numeric_value(y2, player)
            )));
        }
        Datum::Point(arr) => {
            let x = player.get_datum(&arr[0]);
            let y = player.get_datum(&arr[1]);
            
            map.str_set("type", &safe_js_string("Point"));
            map.str_set("x", &concrete_datum_to_js_bridge(x, player, depth + 1));
            map.str_set("y", &concrete_datum_to_js_bridge(y, player, depth + 1));
            map.str_set("value", &safe_js_string(&format!(
                "point({}, {})",
                format_numeric_value(x, player),
                format_numeric_value(y, player)
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
        }
    }
}
