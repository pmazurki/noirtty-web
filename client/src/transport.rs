//! WebSocket transport for terminal I/O

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};
use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::Rc;
use serde::{Deserialize, Serialize};
use bincode;
use crate::terminal::TerminalFrame;

#[derive(Serialize)]
#[serde(tag = "type")]
enum ClientMessage {
    #[serde(rename = "data")]
    Data { data: String },
    #[serde(rename = "resize")]
    Resize { cols: u16, rows: u16 },
    #[serde(rename = "scroll")]
    Scroll { delta: i32 },
    #[serde(rename = "quality")]
    Quality { min_interval_ms: u32 },
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum ServerMessage {
    #[serde(rename = "frame")]
    Frame(TerminalFrame),
}

/// WebSocket transport
pub struct Transport {
    ws: WebSocket,
    recv_buffer: Rc<RefCell<VecDeque<TerminalFrame>>>,
    max_frames: Rc<Cell<usize>>,
    bytes_received: Rc<Cell<u64>>,
    messages_received: Rc<Cell<u64>>,
}

impl Transport {
    /// Connect to WebSocket server
    pub async fn connect(url: &str) -> Result<Self, JsValue> {
        let ws = WebSocket::new(url)?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let recv_buffer = Rc::new(RefCell::new(VecDeque::new()));
        let max_frames = Rc::new(Cell::new(8));
        let bytes_received = Rc::new(Cell::new(0_u64));
        let messages_received = Rc::new(Cell::new(0_u64));

        // Wait for connection
        let ws_clone = ws.clone();
        let open_promise = js_sys::Promise::new(&mut |resolve, reject| {
            let ws = ws_clone.clone();
            let onopen = Closure::once(Box::new(move || {
                resolve.call0(&JsValue::NULL).unwrap();
            }) as Box<dyn FnOnce()>);

            let onerror = Closure::once(Box::new(move |_: JsValue| {
                reject.call0(&JsValue::NULL).unwrap();
            }) as Box<dyn FnOnce(JsValue)>);

            ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));
            ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));

            onopen.forget();
            onerror.forget();
        });

        wasm_bindgen_futures::JsFuture::from(open_promise).await?;

        // Setup message handler
        let buffer = recv_buffer.clone();
        let max_frames_ref = max_frames.clone();
        let bytes_ref = bytes_received.clone();
        let messages_ref = messages_received.clone();
        let onmessage = Closure::wrap(Box::new(move |e: MessageEvent| {
            let limit = max_frames_ref.get();
            messages_ref.set(messages_ref.get().wrapping_add(1_u64));
            if limit > 0 {
                // Drop early to avoid JSON parse when we're already behind.
                if buffer.borrow().len() >= limit {
                    return;
                }
            }
            let data = e.data();
            if let Some(text) = data.as_string() {
                bytes_ref.set(bytes_ref.get().wrapping_add(text.len() as u64));
                if let Ok(msg) = serde_json::from_str::<ServerMessage>(&text) {
                    match msg {
                        ServerMessage::Frame(frame) => {
                            let mut buf = buffer.borrow_mut();
                            if limit > 0 {
                                while buf.len() >= limit {
                                    buf.pop_front();
                                }
                            }
                            buf.push_back(frame);
                        }
                    }
                }
                return;
            }

            // Binary (bincode) path.
            if let Ok(array_buf) = data.dyn_into::<js_sys::ArrayBuffer>() {
                let bytes = js_sys::Uint8Array::new(&array_buf).to_vec();
                bytes_ref.set(bytes_ref.get().wrapping_add(bytes.len() as u64));
                if let Ok(msg) = bincode::deserialize::<ServerMessage>(&bytes) {
                    match msg {
                        ServerMessage::Frame(frame) => {
                            let mut buf = buffer.borrow_mut();
                            if limit > 0 {
                                while buf.len() >= limit {
                                    buf.pop_front();
                                }
                            }
                            buf.push_back(frame);
                        }
                    }
                }
            }
        }) as Box<dyn FnMut(MessageEvent)>);

        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
        onmessage.forget();

        Ok(Transport { ws, recv_buffer, max_frames, bytes_received, messages_received })
    }

    /// Send data to terminal
    pub fn send(&self, data: &[u8]) -> Result<(), JsValue> {
        let msg = ClientMessage::Data {
            data: String::from_utf8_lossy(data).into_owned(),
        };
        let json = serde_json::to_string(&msg).map_err(|e| JsValue::from_str(&e.to_string()))?;
        self.ws.send_with_str(&json)
    }

    /// Send resize command
    pub fn send_resize(&self, cols: u16, rows: u16) -> Result<(), JsValue> {
        let msg = ClientMessage::Resize { cols, rows };
        let json = serde_json::to_string(&msg).map_err(|e| JsValue::from_str(&e.to_string()))?;
        self.ws.send_with_str(&json)
    }

    /// Send scroll command (positive = scroll up).
    pub fn send_scroll(&self, delta: i32) -> Result<(), JsValue> {
        let msg = ClientMessage::Scroll { delta };
        let json = serde_json::to_string(&msg).map_err(|e| JsValue::from_str(&e.to_string()))?;
        self.ws.send_with_str(&json)
    }

    /// Limit the number of frames kept in the client queue (0 = unlimited).
    pub fn set_max_frames(&self, max_frames: usize) {
        self.max_frames.set(max_frames);
    }

    /// Throttle server frame rate (0 = no throttle).
    pub fn send_quality(&self, min_interval_ms: u32) -> Result<(), JsValue> {
        let msg = ClientMessage::Quality { min_interval_ms };
        let json = serde_json::to_string(&msg).map_err(|e| JsValue::from_str(&e.to_string()))?;
        self.ws.send_with_str(&json)
    }

    /// Try to receive data
    pub fn try_recv(&self) -> Option<TerminalFrame> {
        self.recv_buffer.borrow_mut().pop_front()
    }

    pub fn queue_len(&self) -> usize {
        self.recv_buffer.borrow().len()
    }

    pub fn bytes_received(&self) -> u64 {
        self.bytes_received.get()
    }

    pub fn messages_received(&self) -> u64 {
        self.messages_received.get()
    }

    pub fn reset_counters(&self) {
        self.bytes_received.set(0);
        self.messages_received.set(0);
    }

    /// WebSocket ready state (0..=3)
    pub fn ready_state(&self) -> u16 {
        self.ws.ready_state()
    }
}

impl Drop for Transport {
    fn drop(&mut self) {
        let _ = self.ws.close();
    }
}
