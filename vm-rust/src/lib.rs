pub mod io;
pub mod js_api;
pub mod player;
pub mod rendering;
pub mod rendering_gpu;
pub mod utils;

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
    eval::eval_lingo_command,
    init_player, reserve_player_mut, reserve_player_ref, PLAYER_OPT,
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
pub async fn load_movie_file(path: String, autoplay: bool) {
    player_dispatch(PlayerVMCommand::LoadMovieFromFile(path, autoplay));
}

// Player control commands bypass the command queue to allow stopping/resetting
// while a breakpoint is active.

#[wasm_bindgen]
pub fn play() {
    reserve_player_mut(|player| {
        player.play();
    });
}

#[wasm_bindgen]
pub fn stop() {
    reserve_player_mut(|player| {
        player.stop();
    });
}

#[wasm_bindgen]
pub fn reset() {
    reserve_player_mut(|player| {
        player.reset();
    });
}

// Debug commands bypass the command queue to avoid deadlocks when a breakpoint
// is hit during command processing. These operations are safe to call directly
// because they only modify player state synchronously.

#[wasm_bindgen]
pub fn add_breakpoint(script_name: String, handler_name: String, bytecode_index: usize) {
    reserve_player_mut(|player| {
        player.breakpoint_manager.add_breakpoint(
            script_name,
            handler_name,
            bytecode_index,
        );
    });
}

#[wasm_bindgen]
pub fn remove_breakpoint(script_name: String, handler_name: String, bytecode_index: usize) {
    reserve_player_mut(|player| {
        player.breakpoint_manager.remove_breakpoint(
            script_name,
            handler_name,
            bytecode_index,
        );
    });
}

#[wasm_bindgen]
pub fn toggle_breakpoint(script_name: String, handler_name: String, bytecode_index: usize) {
    reserve_player_mut(|player| {
        player.breakpoint_manager.toggle_breakpoint(
            script_name,
            handler_name,
            bytecode_index,
        );
    });
}

#[wasm_bindgen]
pub fn resume_breakpoint() {
    reserve_player_mut(|player| {
        player.resume_breakpoint();
    });
}

#[wasm_bindgen]
pub fn step_into() {
    reserve_player_mut(|player| {
        player.step_into();
    });
}

#[wasm_bindgen]
pub fn step_over() {
    reserve_player_mut(|player| {
        player.step_over();
    });
}

#[wasm_bindgen]
pub fn step_out() {
    reserve_player_mut(|player| {
        player.step_out();
    });
}

#[wasm_bindgen]
pub fn step_over_line(skip_bytecode_indices: Vec<usize>) {
    reserve_player_mut(|player| {
        player.step_over_line(skip_bytecode_indices);
    });
}

#[wasm_bindgen]
pub fn step_into_line(skip_bytecode_indices: Vec<usize>) {
    reserve_player_mut(|player| {
        player.step_into_line(skip_bytecode_indices);
    });
}

#[wasm_bindgen]
pub fn set_break_on_error(enabled: bool) {
    reserve_player_mut(|player| {
        player.break_on_error = enabled;
    });
}

#[wasm_bindgen]
pub fn get_break_on_error() -> bool {
    reserve_player_ref(|player| player.break_on_error)
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

// Inspector commands bypass the command queue to allow inspecting state
// while a breakpoint is active.

#[wasm_bindgen]
pub fn request_datum(datum_id: u32) {
    reserve_player_ref(|player| {
        if let Some(datum_ref) = player.allocator.get_datum_ref(datum_id as DatumId) {
            JsApi::dispatch_datum_snapshot(&datum_ref, player);
        }
    });
}

#[wasm_bindgen]
pub fn request_script_instance_snapshot(script_instance_id: u32) {
    reserve_player_ref(|player| {
        JsApi::dispatch_script_instance_snapshot(
            if script_instance_id > 0 {
                Some(
                    player
                        .allocator
                        .get_script_instance_ref(script_instance_id)
                        .unwrap(),
                )
            } else {
                None
            },
            player,
        );
    });
}

#[wasm_bindgen]
pub fn subscribe_to_member(cast_lib: i32, cast_member: i32) {
    let member_ref = cast_member_ref(cast_lib, cast_member);
    reserve_player_mut(|player| {
        if !player.subscribed_member_refs.contains(&member_ref) {
            player.subscribed_member_refs.push(member_ref.clone());
        }
    });
    JsApi::dispatch_cast_member_changed(member_ref);
}

#[wasm_bindgen]
pub fn unsubscribe_from_member(cast_lib: i32, cast_member: i32) {
    let member_ref = cast_member_ref(cast_lib, cast_member);
    reserve_player_mut(|player| {
        player.subscribed_member_refs.retain(|x| x != &member_ref);
    });
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

// Eval command bypasses the command queue to allow evaluating expressions
// while a breakpoint is active (e.g., inspecting variables in the debugger).

#[wasm_bindgen]
pub fn eval_command(command: String) {
    spawn_local(async move {
        JsApi::dispatch_debug_message(&command);
        let result = eval_lingo_command(command).await;
        if let Err(err) = result {
            reserve_player_ref(|player| {
                JsApi::dispatch_script_error(player, &err);
            });
        }
    });
}

/// Check if WebGL2 is supported in the browser
#[wasm_bindgen]
pub fn is_webgl2_supported() -> bool {
    rendering_gpu::is_webgl2_supported()
}

/// Get the current renderer backend name
#[wasm_bindgen]
pub fn get_renderer_backend() -> String {
    use rendering_gpu::Renderer;
    rendering::with_renderer_mut(|renderer_lock| {
        if let Some(renderer) = renderer_lock {
            renderer.backend_name().to_string()
        } else {
            "none".to_string()
        }
    })
}

#[wasm_bindgen(start)]
pub fn main() {
    set_panic_hook();
    init_player();
}
