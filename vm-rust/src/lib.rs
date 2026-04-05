#![allow(static_mut_ref)]
pub mod io;
pub mod js_api;
pub mod player;
pub mod rendering;
pub mod rendering_gpu;
pub mod utils;

use async_std::task::spawn_local;
use log::debug;
use js_api::JsApi;
use num::ToPrimitive;
use utils::set_panic_hook;
use wasm_bindgen::prelude::*;

#[macro_use]
extern crate pest_derive;

pub mod director;

use player::{
    cast_lib::{cast_member_ref, CastMemberRef},
    commands::{player_dispatch, PlayerVMCommand},
    datum_ref::DatumId,
    eval::eval_lingo_command,
    init_player, reserve_player_mut, reserve_player_ref,
    score::get_sprite_at,
    PLAYER_OPT,
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
    // Update mouse state immediately so the mouseH/the mouseV/the stillDown
    // reflect real state even during long-running script handlers
    reserve_player_mut(|player| {
        player.mouse_loc = (x.to_i32().unwrap(), y.to_i32().unwrap());
        player.movie.mouse_down = true;
    });
    player_dispatch(PlayerVMCommand::MouseDown((
        x.to_i32().unwrap(),
        y.to_i32().unwrap(),
    )));
}

#[wasm_bindgen]
pub fn mouse_up(x: f64, y: f64) {
    // Update mouse state immediately so the mouseH/the mouseV/the stillDown
    // reflect real state even during long-running script handlers
    reserve_player_mut(|player| {
        player.mouse_loc = (x.to_i32().unwrap(), y.to_i32().unwrap());
        player.movie.mouse_down = false;
    });
    player_dispatch(PlayerVMCommand::MouseUp((
        x.to_i32().unwrap(),
        y.to_i32().unwrap(),
    )));
}

#[wasm_bindgen]
pub fn mouse_move(x: f64, y: f64) {
    // Update mouse_loc immediately so the mouseH/the mouseV reflect real
    // position even during long-running script handlers (same pattern as key_down/key_up)
    reserve_player_mut(|player| {
        player.mouse_loc = (x.to_i32().unwrap(), y.to_i32().unwrap());
    });
    player_dispatch(PlayerVMCommand::MouseMove((
        x.to_i32().unwrap(),
        y.to_i32().unwrap(),
    )));
}

#[wasm_bindgen]
pub fn key_down(key: String, code: u16) {
    // Update keyboard state immediately so keyPressed() reflects
    // real state even during long-running script handlers
    reserve_player_mut(|player| {
        player.keyboard_manager.key_down(key.clone(), code);
    });
    player_dispatch(PlayerVMCommand::KeyDown(key, code));
}

#[wasm_bindgen]
pub fn key_up(key: String, code: u16) {
    // Update keyboard state immediately so keyPressed() reflects
    // real state even during long-running script handlers
    reserve_player_mut(|player| {
        player.keyboard_manager.key_up(&key, code);
    });
    player_dispatch(PlayerVMCommand::KeyUp(key, code));
}

// Picking mode commands bypass the command queue for synchronous access.

#[wasm_bindgen]
pub fn player_set_picking_mode(enabled: bool) {
    reserve_player_mut(|player| {
        player.picking_mode = enabled;
    });
}

#[wasm_bindgen]
pub fn player_get_sprite_at(x: f64, y: f64) -> i32 {
    reserve_player_ref(|player| {
        get_sprite_at(player, x as i32, y as i32, false)
            .map(|n| n as i32)
            .unwrap_or(0)
    })
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
pub fn get_cast_chunk_list(cast_number: u32) -> JsValue {
    reserve_player_ref(|player| {
        JsApi::get_cast_chunk_list_for(player, cast_number).into()
    })
}

#[wasm_bindgen]
pub fn get_movie_top_level_chunks() -> JsValue {
    reserve_player_ref(|player| {
        JsApi::get_movie_top_level_chunks(player).into()
    })
}

#[wasm_bindgen]
pub fn get_chunk_bytes(cast_number: u32, chunk_id: u32) -> Option<Vec<u8>> {
    reserve_player_ref(|player| {
        JsApi::get_chunk_bytes(player, cast_number, chunk_id)
    })
}

#[wasm_bindgen]
pub fn get_parsed_chunk(cast_number: u32, chunk_id: u32) -> JsValue {
    reserve_player_ref(|player| {
        JsApi::get_parsed_chunk(player, cast_number, chunk_id).into()
    })
}

#[wasm_bindgen]
pub fn clear_debug_messages() {
    reserve_player_mut(|player| {
        player.debug_datum_refs.clear();
    });
}

#[wasm_bindgen]
pub fn set_eval_scope_index(index: i32) {
    reserve_player_mut(|player| {
        player.eval_scope_index = if index >= 0 { Some(index as u32) } else { None };
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

/// Set glyph rendering preference for text/field members.
/// Values: "auto" (default), "bitmap" (PFR atlas), "native" (Canvas2D fillText),
///         "outline" (force outline rasterization, skip PFR bitmap strikes — needs clear_font_cache)
#[wasm_bindgen]
pub fn set_glyph_preference(mode: &str) {
    use player::font::{GlyphPreference, set_glyph_preference as set_pref};
    let pref = match mode.to_lowercase().as_str() {
        "bitmap" => GlyphPreference::Bitmap,
        "native" => GlyphPreference::Native,
        "outline" => GlyphPreference::Outline,
        _ => GlyphPreference::Auto,
    };
    set_pref(pref);
}

/// Get the current glyph rendering preference.
#[wasm_bindgen]
pub fn get_glyph_preference() -> String {
    use player::font::{GlyphPreference, get_glyph_preference as get_pref};
    match get_pref() {
        GlyphPreference::Auto => "auto".to_string(),
        GlyphPreference::Bitmap => "bitmap".to_string(),
        GlyphPreference::Native => "native".to_string(),
        GlyphPreference::Outline => "outline".to_string(),
    }
}

/// Clear the font cache so fonts will be re-rasterized on next use.
/// Call this after set_glyph_preference("outline") to see the effect.
#[wasm_bindgen]
pub fn clear_font_cache() {
    reserve_player_mut(|player| {
        let count = player.font_manager.font_cache.len();
        player.font_manager.font_cache.clear();
        player.font_manager.fonts.clear();
        player.font_manager.font_by_id.clear();
        player.font_manager.font_counter = 0;
        debug!("[clear_font_cache] Cleared {} cached fonts. Reload movie to re-rasterize.", count);
    });
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

// ============================================================================
// MCP (Model Context Protocol) functions for VM debugging
// These functions return JSON strings and are used by the MCP server
// ============================================================================

#[wasm_bindgen]
pub fn mcp_list_scripts(cast_lib: i32, limit: i32, offset: i32) -> String {
    reserve_player_ref(|player| {
        let cast_lib_opt = if cast_lib < 0 { None } else { Some(cast_lib) };
        let limit_opt = if limit < 0 { None } else { Some(limit as usize) };
        let offset_opt = if offset < 0 { None } else { Some(offset as usize) };
        player::mcp::mcp_list_scripts(player, cast_lib_opt, limit_opt, offset_opt)
    })
}

#[wasm_bindgen]
pub fn mcp_get_script(cast_lib: i32, cast_member: i32) -> String {
    reserve_player_ref(|player| player::mcp::mcp_get_script(player, cast_lib, cast_member))
}

#[wasm_bindgen]
pub fn mcp_disassemble_handler(cast_lib: i32, cast_member: i32, handler_name: String) -> String {
    reserve_player_ref(|player| {
        player::mcp::mcp_disassemble_handler(player, cast_lib, cast_member, &handler_name)
    })
}

#[wasm_bindgen]
pub fn mcp_decompile_handler(cast_lib: i32, cast_member: i32, handler_name: String) -> String {
    reserve_player_ref(|player| {
        player::mcp::mcp_decompile_handler(player, cast_lib, cast_member, &handler_name)
    })
}

#[wasm_bindgen]
pub fn mcp_get_call_stack(depth: i32, include_locals: bool) -> String {
    reserve_player_ref(|player| {
        let depth_opt = if depth < 0 { None } else { Some(depth as usize) };
        player::mcp::mcp_get_call_stack(player, depth_opt, include_locals)
    })
}

#[wasm_bindgen]
pub fn mcp_get_context() -> String {
    reserve_player_ref(|player| player::mcp::mcp_get_context(player))
}

#[wasm_bindgen]
pub fn mcp_get_execution_state() -> String {
    reserve_player_ref(|player| player::mcp::mcp_get_execution_state(player))
}

#[wasm_bindgen]
pub fn mcp_get_globals() -> String {
    reserve_player_ref(|player| player::mcp::mcp_get_globals(player))
}

#[wasm_bindgen]
pub fn mcp_get_locals(scope_index: i32) -> String {
    reserve_player_ref(|player| {
        let index = if scope_index < 0 {
            None
        } else {
            Some(scope_index as usize)
        };
        player::mcp::mcp_get_locals(player, index)
    })
}

#[wasm_bindgen]
pub fn mcp_inspect_datum(datum_id: u32) -> String {
    reserve_player_ref(|player| player::mcp::mcp_inspect_datum(player, datum_id as usize))
}

#[wasm_bindgen]
pub fn mcp_list_cast_libs() -> String {
    reserve_player_ref(|player| player::mcp::mcp_list_cast_libs(player))
}

#[wasm_bindgen]
pub fn mcp_get_console_output(last_n_lines: usize) -> String {
    let lines = reserve_player_ref(|player| {
        player
            .console
            .read_tail(last_n_lines) 
    });
    return lines;
}


#[wasm_bindgen]
pub fn mcp_list_cast_members(cast_lib: i32) -> String {
    reserve_player_ref(|player| {
        let lib = if cast_lib < 0 { None } else { Some(cast_lib) };
        player::mcp::mcp_list_cast_members(player, lib)
    })
}

#[wasm_bindgen]
pub fn mcp_inspect_cast_member(cast_lib: i32, cast_member: i32) -> String {
    reserve_player_ref(|player| {
        player::mcp::mcp_inspect_cast_member(player, cast_lib, cast_member)
    })
}

#[wasm_bindgen]
pub fn mcp_list_breakpoints() -> String {
    reserve_player_ref(|player| player::mcp::mcp_list_breakpoints(player))
}

/// Evaluate a Lingo expression and return the result as JSON.
/// Unlike eval_command, this waits for completion and returns the result.
#[wasm_bindgen]
pub async fn mcp_eval_lingo(code: String) -> String {
    let result = eval_lingo_command(code).await;
    reserve_player_ref(|player| player::mcp::mcp_format_eval_result(player, result))
}

#[wasm_bindgen(start)]
pub fn start() {
    set_panic_hook();
    init_player();
}
