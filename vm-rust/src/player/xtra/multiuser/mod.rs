use async_std::{channel::Sender, task::spawn_local};
use fxhash::FxHashMap;
use log::warn;
use wasm_bindgen::{closure::Closure, JsCast};
use web_sys::{ErrorEvent, Event, MessageEvent, WebSocket};

use crate::{
    director::lingo::datum::{Datum, DatumType},
    player::{
        events::player_dispatch_callback_event, reserve_player_mut, reserve_player_ref, DatumRef,
        ScriptError,
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

    pub fn has_instance_async_handler(_name: &String) -> bool {
        false
    }

    pub async fn call_instance_async_handler(
        handler_name: &String,
        instance_id: u32,
        _args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        Err(ScriptError::new(format!(
            "No async handler {} found for Multiuser xtra instance #{}",
            handler_name, instance_id
        )))
    }

    pub fn call_instance_handler(
        handler_name: &String,
        instance_id: u32,
        args: &Vec<DatumRef>,
    ) -> Result<DatumRef, ScriptError> {
        match handler_name.as_str() {
            "setNetBufferLimits" => Ok(DatumRef::Void),
            "setNetMessageHandler" => {
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
            "connectToNetServer" => {
                let mut multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                if let Some((handler_obj_ref, handler_symbol)) = &instance.net_message_handler {
                    let _handler_symbol = handler_symbol.clone();
                    let _handler_obj_ref = handler_obj_ref.clone();
                }
                // userNameString, passwordString, serverIDString, portNumber, movieIDString {, mode, encryptionKey
                let (host, port) = reserve_player_ref(|player| {
                    let host = player.get_datum(args.get(2).unwrap()).string_value()?;
                    let port = player.get_datum(args.get(3).unwrap()).int_value()?;

                    Ok((host, port))
                })?;
                let ws_url = format!("ws://{}:{}", host, port);
                let socket = WebSocket::new(&ws_url).unwrap();
                socket.set_binary_type(web_sys::BinaryType::Arraybuffer);

                let socket_clone = socket.clone();
                let onmessage_callback = Closure::<dyn FnMut(_)>::new(move |e: MessageEvent| {
                    // e.data().dyn_into::<js_sys::JsString>()
                    let data = e.data().dyn_into::<js_sys::ArrayBuffer>().unwrap();
                    let array = js_sys::Uint8Array::new(&data);
                    let vec = array.to_vec();
                    let string = String::from_utf8_lossy(&vec);
                    warn!("WebSocket message: {:?}", string);

                    let mut multiusr_manager =
                        unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                    let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                    instance.dispatch_message(MultiuserMessage {
                        error_code: 0,
                        recipients: vec!["*".to_string()],
                        sender_id: "System".to_string(),
                        subject: "String".to_string(),
                        content: Datum::String(string.to_string()),
                        time_stamp: 0, // TODO timestamp
                    });
                });
                let onerror_callback = Closure::<dyn FnMut(_)>::new(move |e: ErrorEvent| {
                    // e.data().dyn_into::<js_sys::JsString>()
                    warn!("WebSocket error: {:?}", e);
                });
                let onopen_callback = Closure::<dyn FnMut(_)>::new(move |e: Event| {
                    warn!("WebSocket opened");
                    let mut multiusr_manager =
                        unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                    let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
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
                socket.set_onopen(Some(onopen_callback.as_ref().unchecked_ref()));

                let (tx, rx) = async_std::channel::unbounded();
                instance.socket_tx = Some(tx);
                spawn_local(async move {
                    while let Ok(message) = rx.recv().await {
                        warn!("Sending message: {:?}", message);
                        socket_clone
                            .send_with_u8_array(&message.as_bytes())
                            .unwrap();
                    }
                });

                // Forget the callback to keep it alive
                onmessage_callback.forget();
                onerror_callback.forget();
                onopen_callback.forget();

                Ok(DatumRef::Void)
            }
            "getNetMessage" => {
                let mut multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                if let Some(message) = instance.next_message() {
                    reserve_player_mut(|player| {
                        let recipient_refs = message
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
                            vec![
                                (error_code_key, error_code),
                                (recipients_key, recipients),
                                (sender_id_key, sender_id),
                                (subject_key, subject),
                                (content_key, content),
                                (time_stamp_key, time_stamp),
                            ],
                            false,
                        )))
                    })
                } else {
                    Ok(DatumRef::Void)
                }
            }
            "sendNetMessage" => {
                let mut multiusr_manager = unsafe { MULTIUSER_XTRA_MANAGER_OPT.as_mut().unwrap() };
                let instance = multiusr_manager.instances.get_mut(&instance_id).unwrap();
                reserve_player_ref(|player| {
                    let msg_string = player.get_datum(args.get(2).unwrap()).string_value()?;
                    warn!("sendNetMessage: {:?}", msg_string);
                    if let Some(tx) = &instance.socket_tx {
                        tx.try_send(msg_string).unwrap();
                        Ok(DatumRef::Void)
                    } else {
                        Err(ScriptError::new("Socket not connected".to_string()))
                    }
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
