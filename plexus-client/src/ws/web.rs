//! The `ws-web` backend: `web_sys::WebSocket` driven by wasm-bindgen
//! closures.
//!
//! The browser gives an event-callback socket rather than a stream, so
//! there is no reader task here — `onmessage` calls straight into
//! [the shared registry](super::registry), `onclose`/`onerror` call
//! `on_close`, and `WebSocket::send_with_str` is synchronous, which is
//! exactly what the unsubscribe-on-drop path needs.
//!
//! The only asynchronous step is the handshake: `connect` waits on a
//! `futures_channel::oneshot` that `onopen` and the first `onerror`
//! resolve. That is deliberately *not* a `JsFuture` — a plain oneshot keeps
//! this backend's dependency list at `wasm-bindgen` + `web-sys` and works
//! under any wasm executor the consumer already has.

use super::registry::Registry;
use crate::error::TransportError;
use futures_channel::oneshot;
use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{CloseEvent, Event, MessageEvent, WebSocket};

/// The socket plus the closures that must outlive `connect`.
///
/// Dropping the last handle detaches the handlers and closes the socket, so
/// a `WsTransport` that goes out of scope does not leave a live connection
/// calling into freed closures.
#[derive(Debug)]
struct Socket {
    socket: WebSocket,
    // Held only to keep the JS callbacks alive; never called from Rust.
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
}

impl Drop for Socket {
    fn drop(&mut self) {
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        self.socket.set_onerror(None);
        let _ = self.socket.close();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct Conn {
    inner: Rc<Socket>,
}

impl Conn {
    pub(crate) fn send(&self, text: String) -> Result<(), TransportError> {
        if self.inner.socket.ready_state() != WebSocket::OPEN {
            return Err(TransportError::closed("websocket connection is closed"));
        }
        self.inner
            .socket
            .send_with_str(&text)
            .map_err(|e| TransportError::other(format!("websocket send failed: {e:?}")))
    }
}

pub(crate) async fn connect(url: &str, registry: Arc<Registry>) -> Result<Conn, TransportError> {
    let socket =
        WebSocket::new(url).map_err(|e| TransportError::connect(format!("{url}: {e:?}")))?;

    let (opened_tx, opened_rx) = oneshot::channel::<Result<(), TransportError>>();
    let opened_tx = Rc::new(RefCell::new(Some(opened_tx)));

    let on_message = {
        let registry = registry.clone();
        // The reader writes too: a call whose stream was dropped before the
        // acknowledgement arrived is cancelled here, because this callback
        // is where the subscription id first becomes known. Cloning the
        // socket handle is a JS reference, not a Rust ownership cycle.
        let socket = socket.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            if let Some(text) = event.data().as_string() {
                super::trace("←", &text);
                if let Some(frame) = registry.on_text(&text) {
                    super::trace("→", &frame);
                    let _ = socket.send_with_str(&frame);
                }
            }
        })
    };
    socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    let on_close = {
        let registry = registry.clone();
        let opened_tx = opened_tx.clone();
        Closure::<dyn FnMut(CloseEvent)>::new(move |event: CloseEvent| {
            let reason = format!(
                "websocket closed by the peer (code {}, {})",
                event.code(),
                event.reason()
            );
            if let Some(tx) = opened_tx.borrow_mut().take() {
                let _ = tx.send(Err(TransportError::connect(reason.clone())));
            }
            registry.on_close(reason);
        })
    };
    socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    let on_error = {
        let registry = registry.clone();
        let opened_tx = opened_tx.clone();
        Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            // The browser deliberately withholds the reason from script.
            let reason = "websocket error".to_string();
            if let Some(tx) = opened_tx.borrow_mut().take() {
                let _ = tx.send(Err(TransportError::connect(reason.clone())));
            }
            registry.on_close(reason);
        })
    };
    socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));

    let on_open = {
        let opened_tx = opened_tx.clone();
        Closure::<dyn FnMut(Event)>::new(move |_event: Event| {
            if let Some(tx) = opened_tx.borrow_mut().take() {
                let _ = tx.send(Ok(()));
            }
        })
    };
    socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    let conn = Conn {
        inner: Rc::new(Socket {
            socket,
            _on_message: on_message,
            _on_close: on_close,
            _on_error: on_error,
        }),
    };

    let outcome = opened_rx
        .await
        .unwrap_or_else(|_| Err(TransportError::connect("websocket never opened")));
    // `onopen` has fired (or will never fire); the handshake closure is done.
    conn.inner.socket.set_onopen(None);
    drop(on_open);
    outcome?;

    Ok(conn)
}
