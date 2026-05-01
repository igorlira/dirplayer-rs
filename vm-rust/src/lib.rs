#![allow(static_mut_ref)]
pub mod io;
pub mod js_api;
pub mod player;
pub mod rendering;
pub mod rendering_gpu;
pub mod utils;

use async_std::task::spawn_local;
use log::{debug, warn};
use js_api::JsApi;
use num::ToPrimitive;
use utils::set_panic_hook;
use wasm_bindgen::prelude::*;

#[macro_use]
extern crate pest_derive;

pub mod director;

use player::{
    cast_lib::{cast_member_ref, CastMemberRef},
    cast_member::CastMemberType,
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
pub fn set_movie_path_override(path: String) {
    player_dispatch(PlayerVMCommand::SetMoviePathOverride(path));
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

/// Returns the trace log file path and content as a JS object { path, content },
/// or null if no trace log file is set or empty.
#[wasm_bindgen]
pub fn get_trace_log() -> JsValue {
    use player::xtra::fileio::FILEIO_XTRA_MANAGER_OPT;

    let (path, data) = reserve_player_ref(|player| {
        let path = player.movie.trace_log_file.clone();
        if path.is_empty() {
            return (String::new(), Vec::new());
        }
        let manager = unsafe { FILEIO_XTRA_MANAGER_OPT.as_ref() };
        let data = manager
            .and_then(|mgr| mgr.virtual_fs.get(&path))
            .cloned()
            .unwrap_or_default();
        (path, data)
    });

    if path.is_empty() || data.is_empty() {
        return JsValue::NULL;
    }

    let obj = js_sys::Object::new();
    let _ = js_sys::Reflect::set(&obj, &"path".into(), &path.into());
    let content = String::from_utf8_lossy(&data);
    let _ = js_sys::Reflect::set(&obj, &"content".into(), &content.as_ref().into());
    obj.into()
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

/// Dump every authored child sprite inside a filmloop member to the browser
/// console. Lingo can't reach a filmloop's child sprites directly (the
/// member.media is opaque), so this exposes the parsed score state
/// dirplayer-rs already holds in memory. Resolves keyframes the same way
/// `render_filmloop_from_channel_data` does (most-recent frame_idx per
/// channel up to current_frame). Call from the JS console:
///   `vm.player_print_filmloop_sprites(2, 145)` for the spiderweb filmloop.
#[wasm_bindgen]
pub fn player_print_filmloop_sprites(cast_lib: i32, cast_member: i32) {
    use crate::player::{cast_member::CastMemberType, reserve_player_ref};
    use crate::player::score::get_channel_number_from_index;
    reserve_player_ref(|player| {
        let member_ref = CastMemberRef { cast_lib, cast_member };
        let Some(member) = player.movie.cast_manager.find_member_by_ref(&member_ref) else {
            web_sys::console::warn_1(&format!(
                "[filmloop-dump] member {}:{} not found", cast_lib, cast_member
            ).into());
            return;
        };
        let CastMemberType::FilmLoop(film) = &member.member_type else {
            web_sys::console::warn_1(&format!(
                "[filmloop-dump] member {}:{} ('{}') is not a filmloop",
                cast_lib, cast_member, member.name
            ).into());
            return;
        };

        let frame = film.current_frame;
        let frame_idx_target = frame.saturating_sub(1);
        let init_data = &film.score.channel_initialization_data;

        // Resolve active sprite per channel: most-recent frame_idx <= target
        let mut latest: std::collections::HashMap<u16, (u32, &crate::director::chunks::score::ScoreFrameChannelData)> =
            std::collections::HashMap::new();
        for (frame_idx, channel_idx, data) in init_data.iter() {
            if *channel_idx < 6 { continue; }
            if data.cast_member == 0 { continue; }
            if *frame_idx > frame_idx_target { continue; }
            latest
                .entry(*channel_idx)
                .and_modify(|(f, d)| if *frame_idx > *f { *f = *frame_idx; *d = data; })
                .or_insert((*frame_idx, data));
        }
        let mut entries: Vec<_> = latest.into_iter().collect();
        entries.sort_by_key(|(ch, _)| *ch);

        web_sys::console::warn_1(&format!(
            "[filmloop-dump] member {}:{} '{}' frame={} init_data_entries={} active_channels={}",
            cast_lib, cast_member, member.name,
            frame, init_data.len(), entries.len()
        ).into());

        // Raw dump of ALL channel_init_data entries (no frame/filter), so we
        // can see channels that activate later, "reverse ink" companions, etc.
        let mut all_raw: Vec<_> = init_data.iter().collect();
        all_raw.sort_by_key(|(f, c, _)| (*c, *f));
        for (f, c, d) in all_raw.iter().take(80) {
            let ilib = if d.cast_lib == 65535 { cast_lib } else { d.cast_lib as i32 };
            let nm = player.movie.cast_manager
                .find_filmloop_inner_member(&CastMemberRef { cast_lib: ilib, cast_member: d.cast_member as i32 })
                .map(|m| m.name.clone())
                .unwrap_or_else(|| "<no_member>".into());
            web_sys::console::warn_1(&format!(
                "[filmloop-raw]   f={} ch={} ink={} blend={} member=({},{}) '{}' size={}x{}",
                f, c, d.ink, d.blend, ilib, d.cast_member, nm, d.width, d.height
            ).into());
        }

        for (channel_idx, (frame_idx, data)) in entries {
            let channel_num = get_channel_number_from_index(channel_idx as u32);
            let resolved_lib = if data.cast_lib == 65535 { cast_lib } else { data.cast_lib as i32 };
            let sprite_ref = CastMemberRef { cast_lib: resolved_lib, cast_member: data.cast_member as i32 };
            let inner = player.movie.cast_manager.find_filmloop_inner_member(&sprite_ref);
            let (mname, mtype, bm_info) = match inner {
                Some(m) => {
                    let info = if let CastMemberType::Bitmap(bm) = &m.member_type {
                        let bmp = player.bitmap_manager.get_bitmap(bm.image_ref);
                        match bmp {
                            Some(b) => format!(" bm={}x{} bd={} obd={} use_alpha={} pal={:?}",
                                b.width, b.height, b.bit_depth, b.original_bit_depth,
                                b.use_alpha, b.palette_ref),
                            None => " bm=<no_data>".into(),
                        }
                    } else {
                        String::new()
                    };
                    (m.name.clone(), format!("{:?}", m.member_type.member_type_id()), info)
                }
                None => ("<not found>".into(), "<none>".into(), String::new()),
            };
            web_sys::console::warn_1(&format!(
                "[filmloop-dump]   ch={} ink={} blend={} member=({},{}) '{}' [{}]{} pos=({},{}) size={}x{} flipH={} flipV={} fore/back=({}/{}) color_flag={} kf_frame={}",
                channel_num,
                data.ink,
                data.blend,
                resolved_lib, data.cast_member,
                mname, mtype, bm_info,
                data.pos_x, data.pos_y,
                data.width, data.height,
                data.flip_h(), data.flip_v(),
                data.fore_color, data.back_color,
                data.color_flag,
                frame_idx
            ).into());

            // If the child is itself a filmloop, dump one level deeper so we
            // can see what bitmaps it ultimately contains.
            if let Some(m) = inner {
                if let CastMemberType::FilmLoop(inner_film) = &m.member_type {
                    let inner_target = inner_film.current_frame.saturating_sub(1);
                    let mut inner_latest: std::collections::HashMap<u16, (u32, &crate::director::chunks::score::ScoreFrameChannelData)> =
                        std::collections::HashMap::new();
                    for (f, c, d) in inner_film.score.channel_initialization_data.iter() {
                        if *c < 6 || d.cast_member == 0 || *f > inner_target { continue; }
                        inner_latest
                            .entry(*c)
                            .and_modify(|(ef, ed)| if *f > *ef { *ef = *f; *ed = d; })
                            .or_insert((*f, d));
                    }
                    let mut inner_entries: Vec<_> = inner_latest.into_iter().collect();
                    inner_entries.sort_by_key(|(c, _)| *c);
                    for (ic, (ifr, id)) in inner_entries {
                        let icn = get_channel_number_from_index(ic as u32);
                        let ilib = if id.cast_lib == 65535 { resolved_lib } else { id.cast_lib as i32 };
                        let inested_ref = CastMemberRef { cast_lib: ilib, cast_member: id.cast_member as i32 };
                        let (inn_name, inn_type) = match player.movie.cast_manager.find_filmloop_inner_member(&inested_ref) {
                            Some(im) => (im.name.clone(), format!("{:?}", im.member_type.member_type_id())),
                            None => ("<not found>".into(), "<none>".into()),
                        };
                        web_sys::console::warn_1(&format!(
                            "[filmloop-dump]     >> nested ch={} ink={} member=({},{}) '{}' [{}] size={}x{} fore/back=({}/{}) color_flag={} kf_frame={}",
                            icn, id.ink, ilib, id.cast_member, inn_name, inn_type,
                            id.width, id.height, id.fore_color, id.back_color, id.color_flag, ifr
                        ).into());
                    }
                }
            }
        }
    });
}

#[wasm_bindgen]
pub fn mouse_down(x: f64, y: f64) {
    // Invert the stage auto-scale so mouseH/mouseV land in movie coordinates,
    // matching where sprites live in Lingo-facing state. No-op when scale=1.
    let (mx, my) = reserve_player_ref(|p| crate::player::stage::canvas_to_movie_coords(p, x, y));
    let (ix, iy) = (mx.to_i32().unwrap(), my.to_i32().unwrap());
    reserve_player_mut(|player| {
        player.mouse_loc = (ix, iy);
        player.movie.mouse_down = true;
    });
    player_dispatch(PlayerVMCommand::MouseDown((ix, iy)));
}

#[wasm_bindgen]
pub fn mouse_up(x: f64, y: f64) {
    let (mx, my) = reserve_player_ref(|p| crate::player::stage::canvas_to_movie_coords(p, x, y));
    let (ix, iy) = (mx.to_i32().unwrap(), my.to_i32().unwrap());
    reserve_player_mut(|player| {
        player.mouse_loc = (ix, iy);
        player.movie.mouse_down = false;
    });
    player_dispatch(PlayerVMCommand::MouseUp((ix, iy)));
}

#[wasm_bindgen]
pub fn mouse_move(x: f64, y: f64) {
    let (mx, my) = reserve_player_ref(|p| crate::player::stage::canvas_to_movie_coords(p, x, y));
    let (ix, iy) = (mx.to_i32().unwrap(), my.to_i32().unwrap());
    reserve_player_mut(|player| {
        player.mouse_loc = (ix, iy);
    });
    player_dispatch(PlayerVMCommand::MouseMove((ix, iy)));
}

/// Check if the game wants pointer lock (for FPS mouse look)
#[wasm_bindgen]
pub fn wants_pointer_lock() -> bool {
    reserve_player_ref(|player| player.wants_pointer_lock)
}

/// Mouse move with delta values (for pointer lock mode)
/// The delta is added to the current mouse_loc (which the game resets to center each frame)
#[wasm_bindgen]
pub fn mouse_move_delta(dx: f64, dy: f64) {
    reserve_player_mut(|player| {
        player.mouse_loc.0 -= dx.to_i32().unwrap();
        player.mouse_loc.1 += dy.to_i32().unwrap();
    });
    let (x, y) = reserve_player_ref(|player| player.mouse_loc);
    player_dispatch(PlayerVMCommand::MouseMove((x, y)));
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
        let (mx, my) = crate::player::stage::canvas_to_movie_coords(player, x, y);
        get_sprite_at(player, mx as i32, my as i32, false)
            .map(|n| n as i32)
            .unwrap_or(0)
    })
}

/// Check if a sprite is an editable field member (for mobile keyboard focus)
#[wasm_bindgen]
pub fn is_sprite_editable_field(sprite_id: i32) -> bool {
    reserve_player_ref(|player| {
        let sprite = player.movie.score.get_sprite(sprite_id as i16);
        let member = sprite
            .and_then(|s| s.member.as_ref())
            .and_then(|m| player.movie.cast_manager.find_member_by_ref(m));
        member.map_or(false, |m| match &m.member_type {
            CastMemberType::Field(f) => f.editable,
            _ => false,
        })
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

/// Receive a rendered Flash frame from JavaScript (Ruffle) and store it as a bitmap.
/// This allows Flash content to be composited into the Director stage rendering pipeline.
#[wasm_bindgen]
pub fn update_flash_frame(cast_lib: i32, cast_member: i32, width: u32, height: u32, rgba_data: &[u8]) {
    use player::bitmap::bitmap::{Bitmap, PaletteRef, get_system_default_palette};

    let expected_len = (width * height * 4) as usize;
    if rgba_data.len() != expected_len {
        warn!(
            "update_flash_frame: expected {} bytes, got {}",
            expected_len, rgba_data.len()
        );
        return;
    }

    let mut bitmap = Bitmap::new(
        width as u16,
        height as u16,
        32,
        32,
        8, // alpha depth
        PaletteRef::BuiltIn(get_system_default_palette()),
    );
    bitmap.data = rgba_data.to_vec();
    bitmap.use_alpha = true;

    unsafe {
        if let Some(player) = PLAYER_OPT.as_mut() {
            let key = (cast_lib, cast_member);
            if let Some(&existing_ref) = player.flash_frame_buffers.get(&key) {
                if existing_ref == 0 {
                    // Sentinel value — first real frame, allocate new bitmap
                    let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                    player.flash_frame_buffers.insert(key, bitmap_ref);
                } else {
                    // Replace existing bitmap to reuse the BitmapRef
                    player.bitmap_manager.replace_bitmap(existing_ref, bitmap);
                }
            } else {
                // No entry at all — allocate a new bitmap
                let bitmap_ref = player.bitmap_manager.add_bitmap(bitmap);
                player.flash_frame_buffers.insert(key, bitmap_ref);
            }
        }
    }
}

// Flash-to-Lingo callback mechanism
#[wasm_bindgen]
pub fn trigger_lingo_callback(sprite_num: i32, handler_name: String, args: JsValue) -> bool {
    use director::lingo::datum::Datum;

    let arg_refs = if js_sys::Array::is_array(&args) {
        let array = js_sys::Array::from(&args);
        let mut refs = Vec::new();
        for i in 0..array.length() {
            let item = array.get(i);
            let datum = if let Some(s) = item.as_string() {
                Datum::String(s)
            } else if let Some(n) = item.as_f64() {
                Datum::Float(n)
            } else {
                Datum::Void
            };
            refs.push(player::player_alloc_datum(datum));
        }
        refs
    } else {
        let datum = if let Some(s) = args.as_string() {
            Datum::String(s)
        } else if let Some(n) = args.as_f64() {
            Datum::Float(n)
        } else {
            Datum::Void
        };
        vec![player::player_alloc_datum(datum)]
    };

    player_dispatch_with_result(PlayerVMCommand::TriggerFlashCallback {
        sprite_num,
        handler_name,
        args: arg_refs,
    })
}

/// Convert a JsValue to a DatumRef, handling objects as PropLists
fn js_value_to_datum_ref(item: &JsValue) -> player::datum_ref::DatumRef {
    js_value_to_datum_ref_with_flash(item, 1, 1)
}

fn js_value_to_datum_ref_with_flash(item: &JsValue, flash_cast_lib: i32, flash_cast_member: i32) -> player::datum_ref::DatumRef {
    use director::lingo::datum::{Datum, DatumType};

    if item.is_null() || item.is_undefined() {
        return player::player_alloc_datum(Datum::Void);
    }
    if let Some(s) = item.as_string() {
        return player::player_alloc_datum(Datum::String(s));
    }
    if let Some(n) = item.as_f64() {
        if n.fract() == 0.0 && n >= i32::MIN as f64 && n <= i32::MAX as f64 {
            return player::player_alloc_datum(Datum::Int(n as i32));
        } else {
            return player::player_alloc_datum(Datum::Float(n));
        }
    }
    if let Some(b) = item.as_bool() {
        return player::player_alloc_datum(Datum::Int(if b { 1 } else { 0 }));
    }
    // Check for arrays before objects (arrays are also objects in JS)
    if js_sys::Array::is_array(item) {
        let array = js_sys::Array::from(item);
        let mut items = std::collections::VecDeque::new();
        for i in 0..array.length() {
            let val = array.get(i);
            items.push_back(js_value_to_datum_ref_with_flash(&val, flash_cast_lib, flash_cast_member));
        }
        // Use XmlChildNodes type for 0-based indexing (Flash arrays are 0-based)
        return player::player_alloc_datum(Datum::List(DatumType::XmlChildNodes, items, false));
    }
    if item.is_object() {
        let obj = js_sys::Object::from(item.clone());

        // Check for __dirplayer_stored_path - this is a Flash object reference
        if let Ok(stored_path) = js_sys::Reflect::get(&obj, &JsValue::from_str("__dirplayer_stored_path")) {
            if let Some(path) = stored_path.as_string() {
                let flash_ref = director::lingo::datum::FlashObjectRef::from_path_with_member(&path, flash_cast_lib, flash_cast_member);
                return player::player_alloc_datum(Datum::FlashObjectRef(flash_ref));
            }
        }

        // Convert JS object to PropList
        let entries = js_sys::Object::entries(&obj);
        let mut props: std::collections::VecDeque<(player::datum_ref::DatumRef, player::datum_ref::DatumRef)> = std::collections::VecDeque::new();
        let mut flash_type: Option<String> = None;

        for i in 0..entries.length() {
            let entry = js_sys::Array::from(&entries.get(i));
            let key = entry.get(0).as_string().unwrap_or_default();
            let val = entry.get(1);

            if key == "#type" {
                flash_type = val.as_string();
                continue;
            }

            let key_ref = player::player_alloc_datum(Datum::Symbol(key));
            let val_ref = js_value_to_datum_ref_with_flash(&val, flash_cast_lib, flash_cast_member);
            props.push_back((key_ref, val_ref));
        }

        // Store the type as a #type property if present
        if let Some(t) = flash_type {
            let key_ref = player::player_alloc_datum(Datum::Symbol("#type".to_string()));
            let val_ref = player::player_alloc_datum(Datum::String(t));
            props.push_front((key_ref, val_ref));
        }

        return player::player_alloc_datum(Datum::PropList(props, false));
    }
    player::player_alloc_datum(Datum::Void)
}

#[wasm_bindgen]
pub fn trigger_lingo_callback_on_script(cast_lib: i32, cast_member: i32, handler_name: String, args: String, flash_cast_lib: i32, flash_cast_member: i32) -> bool {
    use director::lingo::datum::Datum;

    let args_js_value = match js_sys::JSON::parse(&args) {
        Ok(val) => val,
        Err(_) => return false,
    };

    let mut arg_refs = Vec::new();

    // Prepend oCaller (the calling object reference) - Director handlers expect this as first arg
    arg_refs.push(player::player_alloc_datum(Datum::Void));

    if js_sys::Array::is_array(&args_js_value) {
        let array = js_sys::Array::from(&args_js_value);
        for i in 0..array.length() {
            let item = array.get(i);
            let datum_ref = js_value_to_datum_ref_with_flash(&item, flash_cast_lib, flash_cast_member);
            arg_refs.push(datum_ref);
        }
    } else {
        return false;
    }


    player_dispatch_with_result(PlayerVMCommand::TriggerLingoCallbackOnScript {
        cast_lib,
        cast_member,
        handler_name,
        args: arg_refs,
    })
}

#[wasm_bindgen]
pub fn set_lingo_script_property(cast_lib: i32, cast_member: i32, prop_name: String, value: JsValue) -> bool {
    use director::lingo::datum::Datum;

    let datum = if let Some(s) = value.as_string() {
        Datum::String(s)
    } else if let Some(n) = value.as_f64() {
        if n.fract() == 0.0 && n >= i32::MIN as f64 && n <= i32::MAX as f64 {
            Datum::Int(n as i32)
        } else {
            Datum::Float(n)
        }
    } else if let Some(b) = value.as_bool() {
        Datum::Int(if b { 1 } else { 0 })
    } else {
        Datum::Void
    };

    let value_ref = player::player_alloc_datum(datum);

    player_dispatch_with_result(PlayerVMCommand::SetLingoScriptProperty {
        cast_lib,
        cast_member,
        prop_name,
        value: value_ref,
    })
}

fn player_dispatch_with_result(command: PlayerVMCommand) -> bool {
    player_dispatch(command);
    true
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

/// Download raw W3D/IFX data for external testing
#[wasm_bindgen(js_name = "exportW3dRaw")]
pub fn export_w3d_raw(cast_lib: i32, cast_member: i32) {
    reserve_player_ref(|player| {
        let member_ref = CastMemberRef { cast_lib, cast_member };
        let member = match player.movie.cast_manager.find_member_by_ref(&member_ref) {
            Some(m) => m,
            None => return,
        };
        let w3d = match member.member_type.as_shockwave3d() {
            Some(w) => w,
            None => return,
        };
        // Find IFX start in the raw data
        let data = &w3d.w3d_data;
        let ifx_magic = [0x49u8, 0x46, 0x58, 0x00];
        let offset = (0..data.len().min(256)).find(|&i| i + 4 <= data.len() && data[i..i+4] == ifx_magic);
        if let Some(off) = offset {
            let ifx_data = &data[off..];
            trigger_browser_download(&format!("member_{}_{}.w3d", cast_lib, cast_member), ifx_data, "application/octet-stream");
            debug!("Exported {} bytes of IFX data (offset {} in {} byte XMED)", ifx_data.len(), off, data.len());
        } else {
            debug!("No IFX magic found in W3D data");
        }
    });
}

#[wasm_bindgen(js_name = "exportW3dObj")]
pub fn export_w3d_obj(cast_lib: i32, cast_member: i32) {
    reserve_player_ref(|player| {
        let member_ref = CastMemberRef { cast_lib, cast_member };
        let member = match player.movie.cast_manager.find_member_by_ref(&member_ref) {
            Some(m) => m,
            None => {
                web_sys::console::error_1(&format!("Member {}:{} not found", cast_lib, cast_member).into());
                return;
            }
        };
        let w3d = match member.member_type.as_shockwave3d() {
            Some(w) => w,
            None => {
                web_sys::console::error_1(&"Not a Shockwave3D member".into());
                return;
            }
        };
        let scene = match &w3d.parsed_scene {
            Some(s) => s,
            None => {
                web_sys::console::error_1(&"No parsed 3D scene".into());
                return;
            }
        };

        let name = if member.name.is_empty() {
            format!("member_{}_{}", cast_lib, cast_member)
        } else {
            member.name.replace(' ', "_")
        };

        // Build ZIP containing OBJ + MTL + GLB + textures
        let mtl_filename = format!("{}.mtl", name);
        let obj_data = scene.export_obj_with_mtl(&mtl_filename);
        let mtl_data = scene.export_mtl(&mtl_filename);
        let glb_data = crate::director::chunks::w3d::gltf_export::export_glb(scene);

        let obj_filename = format!("{}.obj", name);
        let glb_filename = format!("{}.glb", name);
        let zip_data = build_zip_with_glb(
            &obj_filename, obj_data.as_bytes(),
            &mtl_filename, mtl_data.as_bytes(),
            &glb_filename, &glb_data,
            &scene.texture_images,
        );

        trigger_browser_download(&format!("{}.zip", name), &zip_data, "application/zip");

        debug!(
            "Exported {}.obj ({} bytes), {}.mtl ({} bytes), {}.glb ({} bytes), {} textures",
            name, obj_data.len(), name, mtl_data.len(), name, glb_data.len(), scene.texture_images.len()
        );
    });
}

/// List all Shockwave3D members in the movie (for use with exportW3dObj)
#[wasm_bindgen(js_name = "listW3dMembers")]
pub fn list_w3d_members() -> String {
    reserve_player_ref(|player| {
        let mut result = String::new();
        for (lib_idx, cast) in player.movie.cast_manager.casts.iter().enumerate() {
            for (_, member) in cast.members.iter() {
                if member.member_type.as_shockwave3d().is_some() {
                    let line = format!(
                        "castLib {}  member {} \"{}\"  (call wasm.exportW3dObj({}, {}) to download)\n",
                        lib_idx + 1, member.number, member.name,
                        lib_idx + 1, member.number
                    );
                    result.push_str(&line);
                }
            }
        }
        if result.is_empty() {
            result = "No Shockwave3D members found.".to_string();
        }
        debug!("{}", result);
        result
    })
}

/// Build a minimal uncompressed ZIP file containing OBJ + MTL + textures
fn build_zip_with_glb(
    obj_name: &str, obj_data: &[u8],
    mtl_name: &str, mtl_data: &[u8],
    glb_name: &str, glb_data: &[u8],
    textures: &std::collections::HashMap<String, Vec<u8>>,
) -> Vec<u8> {
    let mut files: Vec<(String, &[u8])> = Vec::new();
    files.push((obj_name.to_string(), obj_data));
    files.push((mtl_name.to_string(), mtl_data));
    files.push((glb_name.to_string(), glb_data));

    for (tex_name, image_data) in textures {
        let ext = if image_data.len() >= 2 && image_data[0] == 0xFF && image_data[1] == 0xD8 {
            "jpg"
        } else if image_data.len() >= 2 && image_data[0] == 0x89 && image_data[1] == 0x50 {
            "png"
        } else {
            "bin"
        };
        files.push((format!("{}.{}", tex_name, ext), image_data));
    }

    let mut zip = Vec::new();
    let mut central_dir = Vec::new();
    let mut offsets: Vec<u32> = Vec::new();

    // Write local file headers + data
    for (name, data) in &files {
        offsets.push(zip.len() as u32);
        let name_bytes = name.as_bytes();
        let crc = crc32(data);

        // Local file header (0x04034b50)
        zip.extend_from_slice(&[0x50, 0x4B, 0x03, 0x04]); // signature
        zip.extend_from_slice(&20u16.to_le_bytes()); // version needed
        zip.extend_from_slice(&0u16.to_le_bytes());  // flags
        zip.extend_from_slice(&0u16.to_le_bytes());  // compression (0=stored)
        zip.extend_from_slice(&0u16.to_le_bytes());  // mod time
        zip.extend_from_slice(&0u16.to_le_bytes());  // mod date
        zip.extend_from_slice(&crc.to_le_bytes());   // crc32
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed size
        zip.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncompressed size
        zip.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes()); // name length
        zip.extend_from_slice(&0u16.to_le_bytes());  // extra length
        zip.extend_from_slice(name_bytes);
        zip.extend_from_slice(data);
    }

    // Write central directory
    let cd_offset = zip.len() as u32;
    for (i, (name, data)) in files.iter().enumerate() {
        let name_bytes = name.as_bytes();
        let crc = crc32(data);

        central_dir.extend_from_slice(&[0x50, 0x4B, 0x01, 0x02]); // signature
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version made by
        central_dir.extend_from_slice(&20u16.to_le_bytes()); // version needed
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // flags
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // compression
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // mod time
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // mod date
        central_dir.extend_from_slice(&crc.to_le_bytes());   // crc32
        central_dir.extend_from_slice(&(data.len() as u32).to_le_bytes()); // compressed size
        central_dir.extend_from_slice(&(data.len() as u32).to_le_bytes()); // uncompressed size
        central_dir.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes()); // name length
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // extra length
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // comment length
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // disk number
        central_dir.extend_from_slice(&0u16.to_le_bytes());  // internal attrs
        central_dir.extend_from_slice(&0u32.to_le_bytes());  // external attrs
        central_dir.extend_from_slice(&offsets[i].to_le_bytes()); // local header offset
        central_dir.extend_from_slice(name_bytes);
    }

    zip.extend_from_slice(&central_dir);

    // End of central directory
    zip.extend_from_slice(&[0x50, 0x4B, 0x05, 0x06]); // signature
    zip.extend_from_slice(&0u16.to_le_bytes());  // disk number
    zip.extend_from_slice(&0u16.to_le_bytes());  // cd disk number
    zip.extend_from_slice(&(files.len() as u16).to_le_bytes()); // entries on disk
    zip.extend_from_slice(&(files.len() as u16).to_le_bytes()); // total entries
    zip.extend_from_slice(&(central_dir.len() as u32).to_le_bytes()); // cd size
    zip.extend_from_slice(&cd_offset.to_le_bytes()); // cd offset
    zip.extend_from_slice(&0u16.to_le_bytes());  // comment length

    zip
}

/// Simple CRC32 (used for ZIP file entries)
fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

fn trigger_browser_download(filename: &str, data: &[u8], mime_type: &str) {
    use js_sys::{Array, Uint8Array};
    use wasm_bindgen::JsCast;

    let uint8_array = Uint8Array::new_with_length(data.len() as u32);
    uint8_array.copy_from(data);

    let array = Array::new();
    array.push(&uint8_array.buffer());

    let mut options = web_sys::BlobPropertyBag::new();
    options.type_(mime_type);

    let blob = match web_sys::Blob::new_with_buffer_source_sequence_and_options(&array, &options) {
        Ok(b) => b,
        Err(_) => return,
    };

    let url = match web_sys::Url::create_object_url_with_blob(&blob) {
        Ok(u) => u,
        Err(_) => return,
    };

    let window = match web_sys::window() {
        Some(w) => w,
        None => return,
    };
    let document = match window.document() {
        Some(d) => d,
        None => return,
    };
    let a = match document.create_element("a") {
        Ok(el) => el,
        Err(_) => return,
    };

    let _ = a.set_attribute("href", &url);
    let _ = a.set_attribute("download", filename);
    let _ = a.set_attribute("style", "display:none");

    let body = match document.body() {
        Some(b) => b,
        None => return,
    };
    let _ = body.append_child(&a);

    if let Some(html_el) = a.dyn_ref::<web_sys::HtmlElement>() {
        html_el.click();
    }

    let _ = body.remove_child(&a);
    let _ = web_sys::Url::revoke_object_url(&url);
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
    // In test mode, BrowserTestPlayer::new() handles initialization
    // with fresh state for each test. Skip init_player() here to avoid
    // spawning command/event loops that interfere with the test harness.
    #[cfg(target_arch = "wasm32")]
    {
        let is_test = web_sys::window()
            .and_then(|w| js_sys::Reflect::get(&w, &"__dirplayerTestMode".into()).ok())
            .map_or(false, |v| v.is_truthy());
        if is_test {
            return;
        }
    }
    init_player();
}
