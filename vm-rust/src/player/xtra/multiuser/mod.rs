use std::collections::VecDeque;
use async_std::{channel::Sender, task::spawn_local};
use fxhash::FxHashMap;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

// Use console::warn_1 directly for debugging since log level is set to Error
macro_rules! multiuser_log {
    ($($arg:tt)*) => {
        web_sys::console::warn_1(&format!($($arg)*).into());
    };
}

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        DatumRef, ScriptError, events::player_dispatch_callback_event, reserve_player_mut, reserve_player_ref
    },
};

pub struct MultiuserMessage {
    pub error_code: i32,
    pub recipients: Vec<String>,
    pub sender_id: String,
    pub subject: String,
    pub content: Datum,
    pub time_stamp: i64,
}

pub struct MultiuserXtraInstance {
    pub net_message_handler: Option<(DatumRef, String)>,
    pub message_queue: Vec<MultiuserMessage>,
    pub socket_tx: Option<Sender<String>>,
}

impl MultiuserXtraInstance {
    pub fn dispatch_message_handler(&self) {
        if let Some((handler_obj_ref, handler_symbol)) = &self.net_message_handler {
            let handler_symbol = handler_symbol.clone();
            let handler_obj_ref = handler_obj_ref.clone();
            player_dispatch_callback_event(handler_obj_ref, &handler_symbol, &vec![]);
        }
    }

    pub fn dispatch_message(&mut self, message: MultiuserMessage) {
        self.message_queue.push(message);
        self.dispatch_message_handler();
    }

    pub fn next_message(&mut self) -> Option<MultiuserMessage> {
        if self.message_queue.is_empty() {
            return None;
        }
        Some(self.message_queue.remove(0))
    }
}

pub struct MultiuserXtraManager {
    pub instances: FxHashMap<u32, MultiuserXtraInstance>,
    pub instance_counter: u32,
}

impl MultiuserXtraManager {
    pub fn create_instance(&mut self, _: &Vec<DatumRef>) -> u32 {
        self.instance_counter += 1;
        self.instances.insert(
            self.instance_counter,
            MultiuserXtraInstance {
                net_message_handler: None,
                message_queue: vec![],
                socket_tx: None,
            },
        );
        self.instance_counter
    }

    pub fn has_instance_async_handler(_name: &str) -> bool {
        false
    }

    pub async fn call_instance_async_handler(
        handler_name: &str,
        instance_id: u32,
        _args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        Err(ScriptError::new(format!(
            "No async handler {} found for Multiuser xtra instance #{}",
            handler_name, instance_id
        )))
    }

    pub fn call_instance_handler(
        handler_name: &str,
        instance_id: u32,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.to_lowercase().as_str() {
            "setnetbufferlimits" => Ok(DatumRef::Void),
            "setnetmessagehandler" => {
                let mut multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                reserve_player_mut(|player| {
                    let handler_symbol = player.get_datum(args.get(0).unwrap());
                    let handler_obj_ref = args.get(1).unwrap().clone();
                    // TODO subject and sender
                    if handler_symbol.is_void() {
                        instance.net_message_handler = None;
                    } else {
                        let handler_symbol = handler_symbol.symbol_value()?;
                        instance.net_message_handler = Some((handler_obj_ref, handler_symbol));
                    }

                    // TODO return error code?
                    Ok(player.alloc_datum(Datum::Int(0)))
                })
            }
            "connecttonetserver" => {
                let mut multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                if let Some((handler_obj_ref, handler_symbol)) = &instance.net_message_handler {
                    let _handler_symbol = handler_symbol.clone();
                    let _handler_obj_ref = handler_obj_ref.clone();
                }
                // Director's connectToNetServer has two overloads:
                //   (A) (userName, password, serverID, port, movieID, [mode, encKey])
                //   (B) (serverID, port, propList[#userID, #password, #movieid],
                //        modeSymbol, [encKey])
                // Form B is what ClubMarian / MaidMarian's Connection
                // Script uses. Detect it by sniffing args[2]: a PropList
                // means form B; otherwise assume form A.
                let (username, password, host, port, movie_id) = reserve_player_ref(|player| {
                    let arg2 = args.get(2).map(|d| player.get_datum(d));
                    let is_form_b = matches!(arg2, Some(Datum::PropList(..)));
                    if is_form_b {
                        // Form B: (host, port, plist, mode, [encKey])
                        let host = player.get_datum(args.get(0).unwrap()).string_value()?;
                        let port = player.get_datum(args.get(1).unwrap()).int_value()?;
                        let plist = player.get_datum(args.get(2).unwrap());
                        let lookup_str = |key: &str| -> String {
                            if let Datum::PropList(pairs, _) = plist {
                                for (k, v) in pairs.iter() {
                                    let kd = player.get_datum(k);
                                    let key_matches = match kd {
                                        Datum::Symbol(s) | Datum::String(s) => {
                                            s.eq_ignore_ascii_case(key)
                                        }
                                        _ => false,
                                    };
                                    if key_matches {
                                        return player.get_datum(v).string_value().unwrap_or_default();
                                    }
                                }
                            }
                            String::default()
                        };
                        let username = lookup_str("userID");
                        let password = lookup_str("password");
                        let movie_id = lookup_str("movieid");
                        Ok((username, password, host, port, movie_id))
                    } else {
                        // Form A: (userName, password, serverID, port, movieID, ...)
                        let username = player.get_datum(args.get(0).unwrap()).string_value().unwrap_or_default();
                        let password = player.get_datum(args.get(1).unwrap()).string_value().unwrap_or_default();
                        let host = player.get_datum(args.get(2).unwrap()).string_value()?;
                        let port = player.get_datum(args.get(3).unwrap()).int_value()?;
                        let movie_id = player.get_datum(args.get(4).unwrap()).string_value().unwrap_or_default();
                        Ok((username, password, host, port, movie_id))
                    }
                })?;

                let window_secure = web_sys::window()
                    .and_then(|w| w.location().protocol().ok())
                    .map_or(false, |p| p == "https:");
                let (ws_path, ws_secure) = reserve_player_ref(|player| {
                    (
                        player.external_params.get("multiuser_websocket_path").cloned(),
                        player.external_params.get("multiuser_websocket_ssl")
                            .and_then(|v|{
                                if v.is_empty() { 
                                    return None;
                                } else {
                                    Some(v)
                                }
                            })
                            .map(|v| {
                                // parse bool
                                v == "true" || v == "1"
                            }),
                    )
                });

                // If ws_secure param is set, it overrides the page protocol check for ws_scheme
                let ws_secure = ws_secure.unwrap_or(window_secure);
                let ws_scheme = if ws_secure {
                    "wss"
                } else {
                    "ws"
                };
                let ws_url = format!("{}://{}:{}{}", ws_scheme, host, port, ws_path.unwrap_or_default());
                multiuser_log!("Multiuser: Connecting to WebSocket URL: {} (user={}, movie={})", ws_url, username, movie_id);

                // Check if the page provides a socket proxy mapping via JS
                let ws_url = {
                    let default_url = ws_url.clone();
                    let window = web_sys::window().unwrap();
                    if let Ok(resolver) = js_sys::Reflect::get(&window, &"dirplayerResolveSocketUrl".into()) {
                        if let Some(func) = resolver.dyn_ref::<js_sys::Function>() {
                            if let Ok(result) = func.call2(&wasm_bindgen::JsValue::NULL, &host.as_str().into(), &wasm_bindgen::JsValue::from_f64(port as f64)) {
                                if let Some(url) = result.as_string() {
                                    if !url.is_empty() {
                                        url
                                    } else {
                                        default_url
                                    }
                                } else {
                                    default_url
                                }
                            } else {
                                default_url
                            }
                        } else {
                            default_url
                        }
                    } else {
                        default_url
                    }
                };
                multiuser_log!("Multiuser: Connecting to WebSocket URL: {} (original={}:{}, user={}, movie={})", ws_url, host, port, username, movie_id);

                let socket = match WebSocket::new(&ws_url) {
                    Ok(s) => s,
                    Err(e) => {
                        multiuser_log!("Multiuser: Failed to create WebSocket: {:?}", e);
                        // Dispatch error message to handler
                        instance.dispatch_message(MultiuserMessage {
                            error_code: -1,
                            recipients: vec![],
                            sender_id: "System".to_string(),
                            subject: "ConnectToNetServer".to_string(),
                            content: Datum::String(format!("Failed to create WebSocket: {:?}", e)),
                            time_stamp: 0,
                        });
                        return Ok(DatumRef::Void);
                    }
                };
                socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

                // Log initial socket state
                multiuser_log!("Multiuser: WebSocket created, readyState={}", socket.ready_state());

                let socket_clone = socket.clone();
                let ws_url_clone = ws_url.clone();
                let onmessage_callback = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
                    // Handle both ArrayBuffer and String messages
                    let data = e.data();
                    let message_str = if let Ok(array_buffer) = data.clone().dyn_into::<js_sys::ArrayBuffer>() {
                        let array = js_sys::Uint8Array::new(&array_buffer);
                        let vec = array.to_vec();
                        let char_vec: Vec<char> = vec.into_iter().map(|byte| byte as char).collect();
                        char_vec.into_iter().collect()
                    } else if let Ok(js_string) = data.dyn_into::<js_sys::JsString>() {
                        js_string.as_string().unwrap_or_default()
                    } else {
                        multiuser_log!("Multiuser: Received unknown message type");
                        return;
                    };
                    // multiuser_log!("Multiuser: WebSocket message received: {:?}", message_str);

                    let multiusr_manager =
                        unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut() };
                    if let Some(manager) = multiusr_manager {
                        if let Some(instance) = manager.instances.get_mut(&instance_id) {
                            instance.dispatch_message(MultiuserMessage {
                                error_code: 0,
                                recipients: vec!["*".to_string()],
                                sender_id: "System".to_string(),
                                subject: "String".to_string(),
                                content: Datum::String(message_str),
                                time_stamp: 0, // TODO timestamp
                            });
                        }
                    }
                });

                let onerror_callback = Closure::<dyn FnMut(_)>::new(move |_e: Event| {
                    // Note: WebSocket error events typically don't provide detailed error info.
                    // The ErrorEvent's message/filename/etc. fields are often undefined for WebSocket errors.
                    // We just log a generic error message.
                    multiuser_log!("Multiuser: WebSocket error occurred (connection failed or aborted)");

                    let mut multiusr_manager =
                        unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                    if let Some(instance) = multiusr_manager.instances.get_mut(&instance_id) {
                        instance.dispatch_message(MultiuserMessage {
                            error_code: -1,
                            recipients: vec![],
                            sender_id: "System".to_string(),
                            subject: "ConnectToNetServer".to_string(),
                            content: Datum::String("WebSocket connection error".to_string()),
                            time_stamp: 0,
                        });
                    }
                });

                let onclose_callback = Closure::<dyn FnMut(_)>::new(move |e: CloseEvent| {
                    multiuser_log!("Multiuser: WebSocket closed - code: {}, reason: '{}', wasClean: {}",
                        e.code(), e.reason(), e.was_clean());

                    let mut multiusr_manager =
                        unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                    if let Some(instance) = multiusr_manager.instances.get_mut(&instance_id) {
                        instance.dispatch_message(MultiuserMessage {
                            error_code: e.code() as i32,
                            recipients: vec![],
                            sender_id: "System".to_string(),
                            subject: "DisconnectFromServer".to_string(),
                            content: Datum::String(e.reason()),
                            time_stamp: 0,
                        });
                    }
                });

                let onopen_callback = Closure::<dyn FnMut(_)>::new(move |_e: Event| {
                    multiuser_log!("Multiuser: WebSocket connected to {}", ws_url_clone);
                    let multiusr_manager =
                        unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut() };
                    let Some(manager) = multiusr_manager else { return; };
                    let Some(instance) = manager.instances.get_mut(&instance_id) else { return; };
                    instance.dispatch_message(MultiuserMessage {
                        error_code: 0,
                        recipients: vec!["*".to_string()],
                        sender_id: "System".to_string(),
                        subject: "ConnectToNetServer".to_string(),
                        content: Datum::Void,
                        time_stamp: 0, // TODO timestamp
                    });
                });
                socket.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
                socket.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
                socket.set_onclose(Some(onclose_callback.as_ref().unchecked_ref()));
                socket.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));

                let (tx, rx) = async_std::channel::unbounded();
                instance.socket_tx = Some(tx);
                spawn_local(async move {
                    while let Ok(message) = rx.recv().await {
                        // Check if WebSocket is open (readyState == 1) before sending
                        // readyState: 0 = CONNECTING, 1 = OPEN, 2 = CLOSING, 3 = CLOSED
                        if socket_clone.ready_state() != 1 {
                            multiuser_log!("Multiuser: Cannot send message, WebSocket not open (state={})", socket_clone.ready_state());
                            continue;
                        }
                        // multiuser_log!("Multiuser: Sending message: {:?}", message);
                        if let Err(e) = socket_clone.send_with_u8_array(&message.as_bytes()) {
                            multiuser_log!("Multiuser: Failed to send message: {:?}", e);
                        }
                    }
                });

                // Forget the callbacks to keep them alive
                onmessage_callback.forget();
                onerror_callback.forget();
                onclose_callback.forget();
                onopen_callback.forget();

                Ok(DatumRef::Void)
            }
            "getnetmessage" => {
                let mut multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                if let Some(message) = instance.next_message() {
                    reserve_player_mut(|player| {
                        let recipient_refs: VecDeque<DatumRef> = message
                            .recipients
                            .iter()
                            .map(|recipient| player.alloc_datum(Datum::String(recipient.clone())))
                            .collect();

                        let error_code = player.alloc_datum(Datum::Int(message.error_code));
                        let recipients =
                            player.alloc_datum(Datum::List(DatumType::List, recipient_refs, false));
                        let sender_id = player.alloc_datum(Datum::String(message.sender_id));
                        let subject = player.alloc_datum(Datum::String(message.subject));
                        let content = player.alloc_datum(message.content);
                        let time_stamp = player.alloc_datum(Datum::Int(message.time_stamp as i32)); // TODO: i64

                        let error_code_key =
                            player.alloc_datum(Datum::String("errorCode".to_string()));
                        let recipients_key =
                            player.alloc_datum(Datum::String("recipients".to_string()));
                        let sender_id_key =
                            player.alloc_datum(Datum::String("senderID".to_string()));
                        let subject_key = player.alloc_datum(Datum::String("subject".to_string()));
                        let content_key = player.alloc_datum(Datum::String("content".to_string()));
                        let time_stamp_key =
                            player.alloc_datum(Datum::String("timeStamp".to_string()));

                        Ok(player.alloc_datum(Datum::PropList(
                            VecDeque::from(vec![
                                (error_code_key, error_code),
                                (recipients_key, recipients),
                                (sender_id_key, sender_id),
                                (subject_key, subject),
                                (content_key, content),
                                (time_stamp_key, time_stamp),
                            ]),
                            false,
                        )))
                    })
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "sendnetmessage" => {
                let mut multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                reserve_player_ref(|player| {
                    let msg_string = player.get_datum(args.get(2).unwrap()).string_value()?;
                    // multiuser_log!("sendNetMessage: {:?}", msg_string);
                    if let Some(tx) = &instance.socket_tx {
                        tx.try_send(msg_string).unwrap();
                        Ok(DatumRef::Void)
                    } else {
                        Err(ScriptError::new("Socket not connected".to_string()))
                    }
                })
            }
            "getnetaddresscookie" => {
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String("".to_string())))
                })
            }
            "getnumberwaitingnetmessages" => {
                let multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get(&instance_id).unwrap();
                let count = instance.message_queue.len() as i32;
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(count)))
                })
            }
            "checknetmessages" => {
                // Process pending messages — in our implementation messages are already
                // dispatched via callbacks, so this is effectively a no-op
                Ok(DatumRef::Void)
            }
            "breakconnection" => {
                // Close the WebSocket connection
                let multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                if let Some(instance) = multiusr_manager.instances.get_mut(&instance_id) {
                    // Drop the sender to close the send loop; the WebSocket
                    // close is handled by the browser when the socket is dropped
                    instance.socket_tx = None;
                }
                Ok(DatumRef::Void)
            }
            "getneterrorstring" => {
                // Return a human-readable error string for an error code
                let error_code = reserve_player_ref(|player| {
                    player.get_datum(args.get(0).unwrap()).int_value()
                })?;
                let error_string = match error_code {
                    0 => "No error",
                    -1 => "Connection failed",
                    -2 => "Connection refused",
                    -3 => "Connection timed out",
                    -4 => "Invalid message",
                    -5 => "Not connected",
                    _ => "Unknown error",
                };
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::String(error_string.to_string())))
                })
            }
            "getpeerconnectionlist" => {
                // Return an empty list — we don't track peer connections in the client
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::List(DatumType::List, VecDeque::new(), false)))
                })
            }
            "waitfornetconnection" => {
                // Server-side: start listening for incoming connections
                // In a browser WASM context, we can't act as a server — return error code
                reserve_player_mut(|player| {
                    Ok(player.alloc_datum(Datum::Int(-1)))
                })
            }
            _ => Err(ScriptError::new(format!(
                "No handler {} found for Multiuser xtra instance #{}",
                handler_name, instance_id
            ))),
        }
    }

    pub fn new() -> MultiuserXtraManager {
        MultiuserXtraManager {
            instances: FxHashMap::default(),
            instance_counter: 0,
        }
    }
}

pub fn borrow_multiuser_manager_mut<T>(callback: impl FnOnce(&mut MultiuserXtraManager) -> T) -> T {
    let mut manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
    callback(&mut *manager)
}

// lazy_static! {
//     pub static ref MULTIUSER_XTRA_MANAGER: Arc<Mutex<MultiuserXtraManager>> =
//         Arc::new(Mutex::new(MultiuserXtraManager::new()));
// }

pub static mut MULTIUSER_XTRA_MANAGER_OPT: Option<MultiuserXtraManager> = None;
