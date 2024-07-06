use std::{collections::HashMap, iter::FromIterator, sync::Arc};

use itertools::Itertools;
use js_sys::Array;
use wasm_bindgen::prelude::*;

use crate::{
    director::{
        chunks::script::ScriptChunk,
        enums::ScriptType,
        file::DirectorFile,
        lingo::{datum::Datum, script::ScriptContext},
    }, player::{
        allocator::ScriptInstanceAllocatorTrait, bitmap::bitmap::PaletteRef, cast_lib::CastMemberRef, cast_member::{CastMember, CastMemberType, ScriptMember}, datum_formatting::{format_concrete_datum, format_datum}, datum_ref::{DatumId, DatumRef}, handlers::datum_handlers::cast_member_ref::CastMemberRefHandlers, reserve_player_ref, score::Score, script::ScriptInstanceId, script_ref::ScriptInstanceRef, DirPlayer, ScriptError, PLAYER_LOCK
    }, rendering::RENDERER_LOCK
};

pub fn ascii_safe(string: &str) -> String {
  string.chars().map(|c| {
    match c as u32 {
      9 => '\t',
      10 => '\n',
      13 => '\r',
      32..=126 => c,
      _ => '?',
    }
  }).collect()
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
    map.str_set("message", &JsValue::from_str(&self.message));
    if let Some(script_member_ref) = self.script_member_ref {
      map.str_set("script_member_ref", &script_member_ref.to_js_value());
    } else {
      map.str_set("script_member_ref", &JsValue::NULL);
    }
    if let Some(handler_name) = self.handler_name {
      map.str_set("handler_name", &JsValue::from_str(&handler_name));
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
    map.str_set("script_name", &JsValue::from_str(&self.script_name));
    map.str_set("handler_name", &JsValue::from_str(&self.handler_name));
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
    map.str_set("handler_name", &JsValue::from_str(&self.handler_name));
    
    let locals = js_sys::Map::new();
    for (k, v) in self.locals {
      locals.set(&JsValue::from_str(&k), &v.unwrap().to_js_value());
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
  pub fn onCastListChanged(names: Array);
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
}

pub struct JsApi {}

impl JsApi {
  pub fn dispatch_datum_snapshot(datum_ref: &DatumRef, player: &DirPlayer) {
    let snapshot = datum_to_js_bridge(datum_ref, player, 0);
    onDatumSnapshot(datum_ref.unwrap(), snapshot);
  }
  pub fn dispatch_script_instance_snapshot(script_ref: Option<ScriptInstanceRef>, player: &DirPlayer) {
    let datum = if script_ref.is_none() {
      Datum::Void
    } else {
      Datum::ScriptInstanceRef(script_ref.clone().unwrap())
    };
    let snapshot = concrete_datum_to_js_bridge(&datum, player, 0);
    onScriptInstanceSnapshot(script_ref.unwrap().id, snapshot);
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

    onMovieLoaded(OnMovieLoadedCallbackData {
      version: dir_file.version,
      test_val: test,
    });
  }

  pub fn dispatch_cast_list_changed() {
    let player_arc = Arc::clone(&PLAYER_LOCK);
    async_std::task::spawn_local(async move {
      let player_mutex = player_arc.lock().await;
      let player = player_mutex.as_ref().unwrap();
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
            .map(|x| JsValue::from_str(&x))
            .collect::<Array>(),
      );
    });
  }

  pub fn dispatch_cast_member_list_changed(cast_number: u32) {
    let player_arc = Arc::clone(&PLAYER_LOCK);
    async_std::task::spawn_local(async move {
      let player_mutex = player_arc.lock().await;
      let player = player_mutex.as_ref().unwrap();
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
    let player_arc = Arc::clone(&PLAYER_LOCK);
    async_std::task::spawn_local(async move {
      let player_mutex = player_arc.lock().await;
      let player = player_mutex.as_ref().unwrap();
      let subscribed_members = &player.subscribed_member_refs;
      if !subscribed_members.contains(&member_ref) {
        return;
      }

      let cast = player.movie.cast_manager.get_cast(member_ref.cast_lib as u32).unwrap();
      let member = cast.members.get(&(member_ref.cast_member as u32)).unwrap();
      let member_map = Self::get_member_snapshot(member, cast.lctx.as_ref(), player);

      onCastMemberChanged(member_ref.to_js().to_js_value(), member_map.to_js_object());
    });
  }

  pub fn on_cast_member_name_changed(slot_number: u32) {
    let player_arc = Arc::clone(&PLAYER_LOCK);
    async_std::task::spawn_local(async move {
      let player_mutex = player_arc.lock().await;
      let player = player_mutex.as_ref().unwrap();

      if player.is_subscribed_to_channel_names {
        for channel in player.movie.score.channels.iter() {
          if channel.sprite.member.as_ref().map(|x| CastMemberRefHandlers::get_cast_slot_number(x.cast_lib as u32, x.cast_member as u32)) == Some(slot_number) {
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
    let player_arc = Arc::clone(&PLAYER_LOCK);
    async_std::task::spawn_local(async move {
      let player_mutex = player_arc.lock().await;
      let player = player_mutex.as_ref().unwrap();

      let snapshot = Self::get_score_snapshot(player, &player.movie.score);
      onScoreChanged(snapshot.to_js_object());
    });
  }

  pub fn dispatch_channel_changed(channel: i16) {
    async_std::task::spawn_local(async move {
      let selected_channel = RENDERER_LOCK.with(|x| x.borrow().as_ref().and_then(|y| y.debug_selected_channel_num));
      if selected_channel.is_some() && selected_channel.unwrap() == channel {
        let player_opt = PLAYER_LOCK.lock().await;
        let player = player_opt.as_ref().unwrap();
        let snapshot = Self::get_channel_snapshot(player, &channel);
        onChannelChanged(channel, snapshot.to_js_object());
      }
    });
  }

  pub fn dispatch_frame_changed(frame: u32) {
    onFrameChanged(frame);
  }

  pub fn dispatch_debug_message(message: &str) {
    onDebugMessage(message);
  }

  pub fn get_mini_member_snapshot(member: &CastMember) -> js_sys::Map {
    let member_map = js_sys::Map::new();
    member_map.str_set("name", &JsValue::from_str(&member.name));
    member_map.str_set(
      "type",
      &JsValue::from_str(&member.member_type.type_string()),
    );
    return member_map;
  }

  pub fn get_member_snapshot(member: &CastMember, lctx: Option<&ScriptContext>, player: &DirPlayer) -> js_sys::Map {
    let member_map = js_sys::Map::new();
    member_map.str_set("number", &JsValue::from(member.number));
    member_map.str_set("name", &JsValue::from_str(&member.name));
    member_map.str_set(
      "type",
      &JsValue::from_str(&member.member_type.type_string()),
    );

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
        member_map.str_set(
            "script",
            &Self::get_script_snapshot(&script_data, &script, &lctx).to_js_object(),
        );
      }
      CastMemberType::Bitmap(bitmap_data) => {
        let bitmap = player.bitmap_manager.get_bitmap(bitmap_data.image_ref).unwrap();
        member_map.str_set("width", &JsValue::from(bitmap.width));
        member_map.str_set("height", &JsValue::from(bitmap.height));
        member_map.str_set("bitDepth", &JsValue::from(bitmap.bit_depth));
        member_map.str_set("paletteRef", &bitmap.palette_ref.to_js_value());
        member_map.str_set("regX", &JsValue::from(bitmap_data.reg_point.0));
        member_map.str_set("regY", &JsValue::from(bitmap_data.reg_point.1));
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

  pub fn get_score_snapshot(_: &DirPlayer, score: &Score) -> js_sys::Map {
    let member_map = js_sys::Map::new();
    member_map.str_set("channelCount", &JsValue::from(score.get_channel_count()));

    member_map.str_set(
      "scriptReferences",
      &js_sys::Array::from_iter(score.script_references.iter().map(|scr_ref| {
        let script_ref_map = js_sys::Map::new();
        script_ref_map.str_set("startFrame", &scr_ref.start_frame.to_js_value());
        script_ref_map.str_set("endFrame", &scr_ref.end_frame.to_js_value());
        script_ref_map.str_set("castLib", &scr_ref.cast_lib.to_js_value());
        script_ref_map.str_set("castMember", &scr_ref.cast_member.to_js_value());
        script_ref_map.to_js_object()
      })),
    );

    return member_map;
  }

  pub fn dispatch_channel_name_changed(channel: i16) {
    async_std::task::spawn_local(async move {
      let player_opt = PLAYER_LOCK.lock().await;
      let player = player_opt.as_ref().unwrap();
      
      if player.is_subscribed_to_channel_names {
        let display_name = Self::get_channel_display_name(&channel, player).unwrap_or("".to_owned());
        onChannelDisplayNameChanged(channel, &display_name);
      }
    });
  }

  fn get_channel_display_name(channel: &i16, player: &DirPlayer) -> Option<String> {
    let channel = player.movie.score.get_channel(*channel);
    let member_ref = &channel.sprite.member.as_ref();
    if member_ref.is_none() || !member_ref.unwrap().is_valid() {
      return None
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
    let display_name = Self::get_channel_display_name(channel_num, player).unwrap_or("".to_owned());

    let member_ref_array = js_sys::Array::new();
    member_ref_array.push(&JsValue::from_f64(member_ref.cast_lib as f64));
    member_ref_array.push(&JsValue::from_f64(member_ref.cast_member as f64));

    let script_instance_array = js_sys::Array::new();
    for script_instance in &channel.sprite.script_instance_list {
      script_instance_array.push(&JsValue::from_f64(script_instance.id as f64));
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
    sprite_map.str_set("bgColor", &channel.sprite.bg_color.to_string().to_js_value());
    sprite_map.str_set("ink", &JsValue::from_f64(channel.sprite.ink as f64));
    sprite_map.str_set("blend", &JsValue::from_f64(channel.sprite.blend as f64));

    return sprite_map;
  }

  pub fn get_script_snapshot(
    member: &ScriptMember,
    chunk: &ScriptChunk,
    lctx: &ScriptContext,
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

    let handlers_array = js_sys::Array::new();
    for handler in &chunk.handlers {
      let handler_map = js_sys::Map::new();
      let bytecode_array = js_sys::Array::new();
      let args_array = js_sys::Array::new();
      let name = &lctx.names[handler.name_id as usize];

      for bytecode in &handler.bytecode_array {
        let bytecode_map = js_sys::Map::new();

        bytecode_map.str_set("pos", &JsValue::from(bytecode.pos));
        bytecode_map.str_set("text", &bytecode.to_bytecode_text(lctx, &handler).to_js_value());

        bytecode_array.push(&bytecode_map.to_js_object());
      }

      for arg in &handler.argument_name_ids {
        args_array.push(&lctx.names[*arg as usize].to_js_value());
      }

      handler_map.str_set("name", &name.to_js_value());
      handler_map.str_set("args", &args_array);
      handler_map.str_set("bytecode", &bytecode_array);
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
        .map(|scope| {

          let scope = JsBridgeScope {
            script_member_ref: scope.script_ref.to_js(),
            bytecode_index: scope.bytecode_index as u32,
            handler_name: scope.handler_ref.1.to_owned(),
            locals: scope.locals.clone(),
            stack: scope.stack.clone(),
            args: scope.args.clone()
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
      globals.set(
        &JsValue::from_str(&k.to_string()),
        &v.unwrap().to_js_value(),
      );
    }
    onGlobalListChanged(globals.to_js_object());
  }

  pub fn dispatch_debug_update(player: &DirPlayer) {
    Self::dispatch_scope_list(player);
    Self::dispatch_global_list(player);
  }

  pub fn dispatch_script_error(player: &DirPlayer, err: &ScriptError) {
    let current_scope = player.scopes.last();
    let data: js_sys::Map = OnScriptErrorCallbackData {
      message: err.message.to_owned(),
      script_member_ref: current_scope.map(|x| x.handler_ref.0.to_js()),
      handler_name: current_scope.map(|x| x.handler_ref.1.to_owned()),
    }.into();

    Self::dispatch_debug_update(player);
    onScriptError(data.to_js_object());
  }

  pub fn dispatch_breakpoint_list_changed() {
    async_std::task::spawn_local(async move {
      let player_opt = PLAYER_LOCK.lock().await;
      let player = player_opt.as_ref().unwrap();
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
    self.set(&JsValue::from_str(key), value);
  }
}

fn datum_to_js_bridge(datum_ref: &DatumRef, player: &DirPlayer, depth: u8) -> JsBridgeDatum {
  let datum = player.get_datum(datum_ref);
  concrete_datum_to_js_bridge(datum, player, depth)
}

fn concrete_datum_to_js_bridge(datum: &Datum, player: &DirPlayer, depth: u8) -> JsBridgeDatum {
  if depth > 20 {
    let map = js_sys::Map::new();
    map.str_set("debugDescription", &JsValue::from_str("TOO DEEP"));
    return map.to_js_object();
  }
  let map = js_sys::Map::new();
  let formatted_value = format_concrete_datum(datum, player);
  map.str_set("debugDescription", &ascii_safe(&formatted_value).to_js_value());
  match datum {
    Datum::String(val) => {
      map.str_set("type", &JsValue::from_str("string"));
      map.str_set("value", &JsValue::from_str(&ascii_safe(val)));
    }
    Datum::Int(val) => {
      map.str_set("type", &JsValue::from_str("number"));
      map.str_set("value", &JsValue::from_f64(*val as f64));
    }
    Datum::Symbol(val) => {
      map.str_set("type", &JsValue::from_str("symbol"));
      map.str_set("value", &JsValue::from_str(val));
    }
    Datum::List(_, item_refs, _) => {
      map.str_set("type", &JsValue::from_str("list"));
      map.str_set("items", &item_refs.iter().map(|x| x.unwrap()).collect_vec().to_js_value());
    }
    Datum::VarRef(_) => {
      map.str_set("type", &JsValue::from_str("var_ref"));
    }
    Datum::Float(val) => {
      map.str_set("type", &JsValue::from_str("number"));
      map.str_set("value", &JsValue::from_f64(*val as f64));
    }
    Datum::Void => {
      map.str_set("type", &JsValue::from_str("void"));
    }
    Datum::CastLib(val) => {
      map.str_set("type", &JsValue::from_str("castLib"));
      map.str_set("value", &JsValue::from_f64(*val as f64));
    }
    Datum::Stage => {
      map.str_set("type", &JsValue::from_str("stage"));
    }
    Datum::PropList(properties, sorted) => {
      map.str_set("type", &JsValue::from_str("propList"));
      let props_map = js_sys::Map::new();
      for (k, v) in properties.iter() {
        let key_str = format_datum(k, player);
        props_map.set(&JsValue::from_str(&key_str), &v.unwrap().to_js_value());
      }
      map.str_set("properties", &props_map.to_js_object());
      map.str_set("sorted", &JsValue::from_bool(*sorted));
    }
    Datum::StringChunk(..) => {
      map.str_set("type", &JsValue::from_str("stringChunk"));
    }
    Datum::ScriptRef(_) => {
      map.str_set("type", &JsValue::from_str("scriptRef"));
    }
    Datum::ScriptInstanceRef(instance_id) => {
      map.str_set("type", &JsValue::from_str("scriptInstance"));
      let instance = player.allocator.get_script_instance(&instance_id);
      let ancestor_id = &instance.ancestor;
      match ancestor_id {
        Some(ancestor_id) => {
          map.str_set("ancestor", &ancestor_id.id.to_js_value());
        }
        None => map.str_set("ancestor", &JsValue::NULL)
      }

      let props_map = js_sys::Map::new();
      for (k, v) in instance.properties.iter() {
        props_map.set(&JsValue::from_str(k), &v.unwrap().to_js_value());
      }
      map.str_set("properties", &props_map.to_js_object());
    }
    Datum::CastMember(_) => {
      map.str_set("type", &JsValue::from_str("castMember"));
    }
    Datum::SpriteRef(_) => {
      map.str_set("type", &JsValue::from_str("spriteRef"));
    }
    Datum::IntRect(..) => {
      map.str_set("type", &JsValue::from_str("intRect"));
    }
    Datum::IntPoint(..) => {
      map.str_set("type", &JsValue::from_str("intPoint"));
    }
    Datum::CursorRef(_) => {
      map.str_set("type", &JsValue::from_str("cursorRef"));
    }
    Datum::TimeoutRef(_) => {
      map.str_set("type", &JsValue::from_str("timeout"));
    }
    Datum::ColorRef(_) => {
      map.str_set("type", &JsValue::from_str("colorRef"));
    }
    Datum::BitmapRef(_) => {
      map.str_set("type", &JsValue::from_str("bitmapRef"));
    }
    Datum::PaletteRef(_) => {
      map.str_set("type", &JsValue::from_str("paletteRef"));
    }
    Datum::Xtra(_) => {
      map.str_set("type", &JsValue::from_str("xtra"));
    }
    Datum::XtraInstance(..) => {
      map.str_set("type", &JsValue::from_str("xtraInstance"));
    }
    Datum::Matte(..) => {
      map.str_set("type", &JsValue::from_str("matte"));
    }
    Datum::Null => {
      map.str_set("type", &JsValue::from_str("null"));
    }
    Datum::PlayerRef => {
      map.str_set("type", &JsValue::from_str("playerRef"));
    }
    Datum::MovieRef => {
      map.str_set("type", &JsValue::from_str("movieRef"));
    }
  }
  return map.to_js_object();
}

pub trait ToJsValue {
  fn to_js_value(&self) -> JsValue;
}

impl ToJsValue for String {
  fn to_js_value(&self) -> JsValue {
    JsValue::from_str(self)
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

impl ToJsValue for PaletteRef {
  fn to_js_value(&self) -> JsValue {
    match self {
      PaletteRef::BuiltIn(id) => JsValue::from_str(&id.symbol_string()),
      PaletteRef::Member(member_ref) => JsValue::from_str(format!("(member {} of castLib {})", member_ref.cast_member, member_ref.cast_lib).as_str()),
    }
  }
}
