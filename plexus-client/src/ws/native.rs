//! The `ws-native` backend: a `tokio-tungstenite` socket, a writer task and
//! a reader task.
//!
//! Why not jsonrpsee's ws-client, which already speaks subscriptions? Two
//! reasons, both about this ticket's contract. Its subscription plumbing is
//! opaque, so "a notification arriving under `menger.call` routes exactly
//! like one under `result`" could not be *pinned by a test* — and it does
//! not build for `wasm32-unknown-unknown`, so the browser half would have
//! been a second, unshared protocol implementation instead of a second
//! socket under [the same routing code](super::registry).

use super::registry::Registry;
use crate::error::TransportError;
use futures_channel::mpsc;
use futures_util::{SinkExt, StreamExt};
use std::sync::Arc;
use tokio_tungstenite::tungstenite::Message;

/// A handle on the writer task's queue.
///
/// Writes are queued rather than awaited so that [`Drop`] can send the
/// unsubscribe frame — a destructor cannot await.
#[derive(Debug, Clone)]
pub(crate) struct Conn {
    outbound: mpsc::UnboundedSender<String>,
}

impl Conn {
    pub(crate) fn send(&self, text: String) -> Result<(), TransportError> {
        self.outbound
            .unbounded_send(text)
            .map_err(|_| TransportError::closed("websocket connection is closed"))
    }
}

pub(crate) async fn connect(url: &str, registry: Arc<Registry>) -> Result<Conn, TransportError> {
    let (socket, _response) = tokio_tungstenite::connect_async(url)
        .await
        .map_err(|e| TransportError::connect(format!("{url}: {e}")))?;

    let (mut sink, mut source) = socket.split();
    let (outbound, mut queue) = mpsc::unbounded::<String>();

    tokio::spawn(async move {
        while let Some(text) = queue.next().await {
            if sink.send(Message::text(text)).await.is_err() {
                break;
            }
        }
        let _ = sink.close().await;
    });

    // The reader writes too: a call whose stream was dropped before the
    // acknowledgement arrived is cancelled here, because this task is where
    // the subscription id first becomes known.
    let reader_outbound = outbound.clone();
    tokio::spawn(async move {
        let write_back = |frame: Option<String>| {
            if let Some(frame) = frame {
                super::trace("→", &frame);
                let _ = reader_outbound.unbounded_send(frame);
            }
        };
        loop {
            match source.next().await {
                Some(Ok(Message::Text(text))) => {
                    super::trace("←", text.as_str());
                    write_back(registry.on_text(text.as_str()));
                }
                // A server is free to frame JSON as binary; jsonrpsee does
                // not, but accepting it costs nothing and a silent drop
                // would be very hard to diagnose.
                Some(Ok(Message::Binary(bytes))) => {
                    if let Ok(text) = std::str::from_utf8(&bytes) {
                        super::trace("←", text);
                        write_back(registry.on_text(text));
                    }
                }
                Some(Ok(Message::Close(_))) | None => {
                    registry.on_close("websocket closed by the peer");
                    return;
                }
                Some(Ok(_)) => {}
                Some(Err(e)) => {
                    registry.on_close(format!("websocket read failed: {e}"));
                    return;
                }
            }
        }
    });

    Ok(Conn { outbound })
}
