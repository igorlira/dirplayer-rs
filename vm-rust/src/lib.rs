mod io;
mod js_api;
mod player;
mod rendering;
mod utils;

use async_std::task::spawn_local;
use js_api::JsApi;
use num::ToPrimitive;
use utils::set_panic_hook;
use wasm_bindgen::prelude::*;

#[macro_use]
extern crate pest_derive;

mod director;

use player::{
    cast_lib::{cast_member_ref, CastMemberRef},
    commands::{player_dispatch, PlayerVMCommand},
    datum_ref::DatumId,
    init_player, reserve_player_ref, PLAYER_OPT,
};

#[wasm_bindgen]
extern "C" {
    fn alert(s: &str);
}

#[wasm_bindgen]
pub fn set_external_params(params: js_sys::Object) {
    let mut external_params = std::collections::HashMap::new();
    let keys = js_sys::Object::keys(&params);
    for key in keys.iter() {
        let key_str = key.as_string().unwrap();
        let value = js_sys::Reflect::get(&params, &key)
            .unwrap()
            .as_string()
            .unwrap();
        external_params.insert(key_str, value);
    }

    player_dispatch(PlayerVMCommand::SetExternalParams(external_params));
}

#[wasm_bindgen]
pub fn set_base_path(path: String) {
    player_dispatch(PlayerVMCommand::SetBasePath(path));
}

#[wasm_bindgen]
pub fn set_system_font_path(path: String) {
    player_dispatch(PlayerVMCommand::SetSystemFontPath(path));
}

#[wasm_bindgen]
pub async fn load_movie_file(path: String) {
    player_dispatch(PlayerVMCommand::LoadMovieFromFile(path));
}

#[wasm_bindgen]
pub fn play() {
    player_dispatch(PlayerVMCommand::Play);
}

#[wasm_bindgen]
pub fn stop() {
    player_dispatch(PlayerVMCommand::Stop);
}

#[wasm_bindgen]
pub fn reset() {
    player_dispatch(PlayerVMCommand::Reset);
}

#[wasm_bindgen]
pub fn add_breakpoint(script_name: String, handler_name: String, bytecode_index: usize) {
    player_dispatch(PlayerVMCommand::AddBreakpoint(
        script_name,
        handler_name,
        bytecode_index,
    ))
}

#[wasm_bindgen]
pub fn remove_breakpoint(script_name: String, handler_name: String, bytecode_index: usize) {
    player_dispatch(PlayerVMCommand::RemoveBreakpoint(
        script_name,
        handler_name,
        bytecode_index,
    ))
}

#[wasm_bindgen]
pub fn toggle_breakpoint(script_name: String, handler_name: String, bytecode_index: usize) {
    player_dispatch(PlayerVMCommand::ToggleBreakpoint(
        script_name,
        handler_name,
        bytecode_index,
    ))
}

#[wasm_bindgen]
pub fn resume_breakpoint() {
    player_dispatch(PlayerVMCommand::ResumeBreakpoint);
}

#[wasm_bindgen]
pub fn set_stage_size(width: u32, height: u32) {
    player_dispatch(PlayerVMCommand::SetStageSize(width, height));
}

#[wasm_bindgen]
pub fn trigger_timeout(name: &str) {
    player_dispatch(PlayerVMCommand::TimeoutTriggered(name.to_string()));
}

#[wasm_bindgen]
pub fn player_print_member_bitmap_hex(cast_lib: i32, cast_member: i32) {
    player_dispatch(PlayerVMCommand::PrintMemberBitmapHex(CastMemberRef {
        cast_lib,
        cast_member,
    }));
}

#[wasm_bindgen]
pub fn mouse_down(x: f64, y: f64) {
    player_dispatch(PlayerVMCommand::MouseDown((
        x.to_i32().unwrap(),
        y.to_i32().unwrap(),
    )));
}

#[wasm_bindgen]
pub fn mouse_up(x: f64, y: f64) {
    player_dispatch(PlayerVMCommand::MouseUp((
        x.to_i32().unwrap(),
        y.to_i32().unwrap(),
    )));
}

#[wasm_bindgen]
pub fn mouse_move(x: f64, y: f64) {
    player_dispatch(PlayerVMCommand::MouseMove((
        x.to_i32().unwrap(),
        y.to_i32().unwrap(),
    )));
}

#[wasm_bindgen]
pub fn key_down(key: String, code: u16) {
    player_dispatch(PlayerVMCommand::KeyDown(key, code));
}

#[wasm_bindgen]
pub fn key_up(key: String, code: u16) {
    player_dispatch(PlayerVMCommand::KeyUp(key, code));
}

#[wasm_bindgen]
pub fn request_datum(datum_id: u32) {
    player_dispatch(PlayerVMCommand::RequestDatum(datum_id as DatumId));
}

#[wasm_bindgen]
pub fn request_script_instance_snapshot(script_instance_ref: u32) {
    player_dispatch(PlayerVMCommand::RequestScriptInstanceSnapshot(
        script_instance_ref,
    ));
}

#[wasm_bindgen]
pub fn subscribe_to_member(cast_lib: i32, cast_member: i32) {
    player_dispatch(PlayerVMCommand::SubscribeToMember(cast_member_ref(
        cast_lib,
        cast_member,
    )));
}

#[wasm_bindgen]
pub fn unsubscribe_from_member(cast_lib: i32, cast_member: i32) {
    player_dispatch(PlayerVMCommand::UnsubscribeFromMember(cast_member_ref(
        cast_lib,
        cast_member,
    )));
}

#[wasm_bindgen]
pub fn trigger_alert_hook() {
    player_dispatch(PlayerVMCommand::TriggerAlertHook);
}

#[wasm_bindgen]
pub fn subscribe_to_channel_names() {
    spawn_local(async {
        let player = unsafe { PLAYER_OPT.as_mut().unwrap() };

        player.is_subscribed_to_channel_names = true;
        for channel in &player.movie.score.channels {
            JsApi::dispatch_channel_name_changed(channel.number as i16);
        }
    });
}

#[wasm_bindgen]
pub fn unsubscribe_from_channel_names() {
    spawn_local(async {
        let player = unsafe { PLAYER_OPT.as_mut().unwrap() };

        player.is_subscribed_to_channel_names = false;
    });
}

#[wasm_bindgen]
pub fn provide_net_task_data(task_id: u32, data: Vec<u8>) {
    // Directly fulfill the task without going through the command queue to avoid deadlock
    // This is safe because we only access the shared state which is behind a mutex
    async_std::task::spawn_local(async move {
        let shared_state_arc =
            reserve_player_ref(|player| std::sync::Arc::clone(&player.net_manager.shared_state));
        let result = Ok(data);
        let mut shared_state = shared_state_arc.lock().await;
        shared_state.fulfill_task(task_id, result).await;
    });
}

#[wasm_bindgen(start)]
pub fn main() {
    set_panic_hook();
    init_player();
}
