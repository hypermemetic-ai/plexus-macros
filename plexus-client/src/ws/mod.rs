//! `WsTransport` — the Plexus WebSocket JSON-RPC transport.
//!
//! One connection, every call multiplexed over it as a JSON-RPC
//! subscription. This is the transport that carries the full
//! [`StreamMetadata`](crate::StreamMetadata) envelope, and the one menger's
//! frontend adopts.
//!
//! ```no_run
//! # async fn demo() -> Result<(), Box<dyn std::error::Error>> {
//! # use plexus_client::{MethodRef, Transport};
//! use plexus_client::WsTransport;
//!
//! let transport = WsTransport::connect("ws://127.0.0.1:4455", "menger").await?;
//! // let client = TicketsClient::new(transport);   // and reuse it for every activation
//! # let _ = transport.call(MethodRef::new("tickets", "list"), serde_json::json!({}));
//! # Ok(())
//! # }
//! ```
//!
//! # The wire
//!
//! Every [`Transport::call`] becomes exactly one subscribe frame:
//!
//! ```json
//! {"jsonrpc":"2.0","id":1,"method":"menger.call",
//!  "params":{"method":"tickets.watch","params":{"epic":"MACRO"}}}
//! ```
//!
//! The reply's `result` **is** the subscription id. Items then arrive as
//! notifications carrying `params.subscription` and `params.result`, and
//! dropping the returned stream sends `menger.call_unsub`.
//!
//! Three properties are worth stating, because getting any of them wrong is
//! silent:
//!
//! - **Routing is by `params.subscription`, never by the notification's
//!   method name.** `PLEXUS_NOTIF_METHOD` is `"result"`, but plexus-core
//!   0.5.3 emits notifications under the subscribe method's own name
//!   (`"menger.call"`). Both spellings route identically here, and both are
//!   pinned by test.
//! - **The subscribe is written when `call` is invoked, not when the
//!   returned stream is first polled.** menger's own `tests/wire.rs` opens a
//!   `watch` and *then* mutates; a lazily-subscribing transport would miss
//!   the mutation and the bug would look like a server problem.
//! - **The item channel exists before the frame is written**, so an item the
//!   server puts on the wire immediately behind the acknowledgement cannot
//!   be lost.
//!
//! # A call with an input half (BIDIR-6)
//!
//! [`Transport::call_with_input`] is a subscribe like any other. The stream
//! it returns also feeds the input: as it is polled it spends the callee's
//! `credit`, sending what is ready as one `<hub>.input {subscription, seq,
//! items}` and `<hub>.end` when the input ends. `credit`, `ack` and
//! `input_closed` are spent here and never yielded. A caller that stops
//! reading stops feeding; no task runs behind its back, so both backends
//! share this.
//!
//! # One type, two backends
//!
//! `plexus-client`'s default feature set is the four-dependency wasm-clean
//! core; a WebSocket needs I/O, and the I/O a browser has is not the I/O a
//! server has. So the transport ships behind two features that expose the
//! **same** `WsTransport` type and the same constructor:
//!
//! | Feature | Backend | For |
//! |---|---|---|
//! | `ws-native` | `tokio-tungstenite` | servers, CLIs, tests |
//! | `ws-web` | `web_sys::WebSocket` | `wasm32-unknown-unknown`, a browser app |
//!
//! Only the socket differs: request ids, the pending table, the routing
//! table, cancellation and both frame shapes live in one place
//! ([`registry`]), so the two backends cannot drift on protocol. Enabling
//! both picks the native one.
//!
//! [`Transport::call`]: crate::Transport::call

mod registry;

// `ws-web` is a browser backend: its socket is a `web_sys::WebSocket`, which
// is `!Send`, while `ItemStream` off wasm32 requires `Send`. Saying so here
// beats a wall of unsatisfied-bound errors inside `Transport::call`.
#[cfg(all(
    feature = "ws-web",
    not(feature = "ws-native"),
    not(target_arch = "wasm32")
))]
compile_error!(
    "plexus-client: feature `ws-web` is the wasm32 browser backend. Off \
     wasm32-unknown-unknown, enable `ws-native` instead (both features \
     expose the same `WsTransport`)."
);

#[cfg(feature = "ws-native")]
#[path = "native.rs"]
mod backend;

#[cfg(all(feature = "ws-web", not(feature = "ws-native")))]
#[path = "web.rs"]
mod backend;

use crate::envelope::StreamItem;
use crate::error::TransportError;
use crate::transport::{InputItems, ItemStream, MethodRef, Transport};
use futures_core::Stream;
use futures_util::{stream, StreamExt};
use registry::{end_frame, input_frame, subscribe_frame, Inbound, InputSignal, Items, Registry, SubSlot};
use serde_json::Value;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

/// Frame tracing, for when a wire question has to be answered with bytes.
///
/// Off unless `PLEXUS_WS_TRACE` is set in the environment; then every frame
/// this transport writes or reads is printed to stderr as `ws → …` / `ws ← …`.
fn trace(direction: &str, frame: &str) {
    use std::sync::OnceLock;
    static ON: OnceLock<bool> = OnceLock::new();
    if *ON.get_or_init(|| std::env::var("PLEXUS_WS_TRACE").is_ok()) {
        eprintln!("ws {direction} {frame}");
    }
}

/// A Plexus client over one WebSocket JSON-RPC connection.
///
/// Cheap to clone and intended to be shared: one connection serves every
/// activation's client, which is the point of multiplexing.
/// `plexus-codegen-rust`'s client opens a fresh connection per call, which
/// cannot hold a `watch` open while issuing a `list`.
#[derive(Debug, Clone)]
pub struct WsTransport {
    conn: backend::Conn,
    registry: Arc<Registry>,
    /// Shuts the socket down when the last handle goes. Streams borrow the
    /// transport, so they are always gone by then.
    _closer: Arc<Closer>,
}

/// Closes the connection when the last [`WsTransport`] handle drops.
///
/// Without it the socket outlives its transport: the reader task holds a
/// writer handle of its own (it answers late acknowledgements with
/// `call_unsub`), so the writer's queue never ends on its own.
#[derive(Debug)]
struct Closer {
    conn: backend::Conn,
}

impl Drop for Closer {
    fn drop(&mut self) {
        self.conn.close();
    }
}

impl WsTransport {
    /// Connect to a Plexus WebSocket endpoint.
    ///
    /// `hub_namespace` is the server's `DynamicHub::new(ns)` namespace —
    /// `"menger"` for menger — **not** an activation namespace. It is
    /// transport configuration because it is a property of the server: the
    /// same `tickets.list` call goes to `menger.call` on one deployment and
    /// to some other hub's `call` on another.
    ///
    /// Fails with [`TransportErrorKind::Connect`] if the socket cannot be
    /// established.
    ///
    /// [`TransportErrorKind::Connect`]: crate::TransportErrorKind::Connect
    pub async fn connect(url: &str, hub_namespace: &str) -> Result<Self, TransportError> {
        let registry = Arc::new(Registry::new(Arc::from(hub_namespace)));
        let conn = backend::connect(url, registry.clone()).await?;
        Ok(Self {
            _closer: Arc::new(Closer { conn: conn.clone() }),
            conn,
            registry,
        })
    }

    /// FRAME-16: [`connect`](Self::connect) over a socket already made —
    /// a page opens it before its wasm is instantiated and hands it over.
    /// Open, it is taken at once; still connecting, when it opens; closing
    /// or closed, refused ([`TransportErrorKind::Connect`]). `ws-web` only.
    ///
    /// [`TransportErrorKind::Connect`]: crate::TransportErrorKind::Connect
    #[cfg(all(feature = "ws-web", not(feature = "ws-native")))]
    pub async fn adopt(socket: web_sys::WebSocket, hub_namespace: &str) -> Result<Self, TransportError> {
        let registry = Arc::new(Registry::new(Arc::from(hub_namespace)));
        let conn = backend::adopt(socket, registry.clone()).await?;
        Ok(Self {
            _closer: Arc::new(Closer { conn: conn.clone() }),
            conn,
            registry,
        })
    }

    /// [`connect`](Self::connect), with headers on the upgrade request — an
    /// `Authorization: Bearer <key proof>` for a server under login, as
    /// synapse signs in. `ws-native` only: a browser's socket takes no
    /// headers (it carries its cookies).
    #[cfg(feature = "ws-native")]
    pub async fn connect_with_headers(
        url: &str,
        hub_namespace: &str,
        headers: &[(String, String)],
    ) -> Result<Self, TransportError> {
        let registry = Arc::new(Registry::new(Arc::from(hub_namespace)));
        let conn = backend::connect_with_headers(url, headers, registry.clone()).await?;
        Ok(Self {
            _closer: Arc::new(Closer { conn: conn.clone() }),
            conn,
            registry,
        })
    }

    /// The hub namespace this transport prefixes `call` / `call_unsub` with.
    pub fn hub_namespace(&self) -> &str {
        self.registry.hub_namespace()
    }

    /// `true` once the socket has gone away. Every stream open at that
    /// moment yielded a [`TransportErrorKind::Closed`] item.
    ///
    /// [`TransportErrorKind::Closed`]: crate::TransportErrorKind::Closed
    pub fn is_closed(&self) -> bool {
        self.registry.is_closed()
    }

    /// Write the subscribe frame and hand back what the returned stream
    /// needs to drain it.
    fn subscribe(&self, method: MethodRef, params: Value) -> Result<Awaiting, TransportError> {
        let request_id = self.registry.next_request_id();
        let slot = SubSlot::new();
        let reply = self.registry.register_pending(request_id, slot.clone())?;

        let frame = subscribe_frame(
            request_id,
            self.registry.hub_namespace(),
            &method.qualified(),
            &params,
        );
        trace("→", &frame);
        if let Err(e) = self.conn.send(frame) {
            self.registry.forget_pending(request_id);
            return Err(e);
        }

        Ok(Awaiting {
            reply,
            // The guard exists from here on, so a stream dropped before its
            // first poll still cancels the subscription it just opened.
            guard: SubGuard {
                conn: self.conn.clone(),
                registry: self.registry.clone(),
                slot,
            },
        })
    }
}

impl Transport for WsTransport {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        // The subscribe goes out NOW, not on first poll — see the module
        // docs: `watch` then mutate is the shape every menger client uses.
        let start = match self.subscribe(method, params) {
            Ok(awaiting) => CallState::Awaiting(awaiting),
            Err(e) => return Box::pin(stream::once(async move { Err(e) })),
        };

        Box::pin(stream::unfold(start, |state| async move {
            match state {
                CallState::Awaiting(awaiting) => match awaiting.established().await {
                    Err(e) => Some((Err(e), CallState::Ended)),
                    Ok(mut active) => active
                        .next_item()
                        .await
                        .map(|item| (item, CallState::Active(active))),
                },
                CallState::Active(mut active) => active
                    .next_item()
                    .await
                    .map(|item| (item, CallState::Active(active))),
                CallState::Ended => None,
            }
        }))
    }

    fn call_with_input<'a>(&'a self, method: MethodRef, params: Value, input: InputItems<'a>) -> ItemStream<'a> {
        let start = match self.subscribe(method, params) {
            Ok(awaiting) => awaiting,
            Err(e) => return Box::pin(stream::once(async move { Err(e) })),
        };
        let feed = Feed { input: Some(input), credit: 0, seq: 0, conn: self.conn.clone(), registry: self.registry.clone(), failed: false };
        Box::pin(stream::unfold(Feeding::Awaiting(start, feed), |state| async move {
            let (mut active, mut feed) = match state {
                Feeding::Awaiting(awaiting, feed) => match awaiting.established().await {
                    Err(e) => return Some((Err(e), Feeding::Ended)),
                    Ok(active) => (active, feed),
                },
                Feeding::Active(active, feed) => (active, feed),
                Feeding::Ended => return None,
            };
            let item = std::future::poll_fn(|cx| feed.poll_next(&mut active, cx)).await?;
            let failed = feed.failed;
            Some((item, if failed { Feeding::Ended } else { Feeding::Active(active, feed) }))
        }))
    }
}

/// [`CallState`], for a call with an input half.
enum Feeding<'a> {
    Awaiting(Awaiting, Feed<'a>),
    Active(Active, Feed<'a>),
    Ended,
}

/// The most items one `<hub>.input` carries, and about the most bytes: a
/// frame is what was ready, and stays well under a server's message cap.
const FRAME_ITEMS: usize = 256;
const FRAME_BYTES: usize = 1024 * 1024;

/// The input half of a call, fed as its stream is polled.
struct Feed<'a> {
    /// `None`: ended, or the callee closed the half.
    input: Option<InputItems<'a>>,
    /// Items the callee has granted and this side has not sent.
    credit: u64,
    /// Frames sent.
    seq: u64,
    conn: backend::Conn,
    registry: Arc<Registry>,
    failed: bool,
}

impl Feed<'_> {
    /// The call's next item: the socket first, and between its items
    /// whatever the input has ready and credit allows.
    fn poll_next(&mut self, active: &mut Active, cx: &mut Context<'_>) -> Poll<Option<Result<StreamItem, TransportError>>> {
        loop {
            let read = match active.items.poll_next_unpin(cx) {
                Poll::Ready(Some(Inbound::Input(InputSignal::Credit(n)))) => {
                    self.credit = self.credit.saturating_add(n);
                    true
                }
                Poll::Ready(Some(Inbound::Input(InputSignal::Ack))) => true,
                Poll::Ready(Some(Inbound::Input(InputSignal::Closed))) => {
                    self.input = None;
                    true
                }
                Poll::Ready(Some(Inbound::Item(item))) => {
                    if let Ok(item) = &item {
                        if item.is_terminal() {
                            self.input = None;
                            active.guard.slot.mark_finished();
                        }
                    }
                    return Poll::Ready(Some(item));
                }
                Poll::Ready(None) => return Poll::Ready(None),
                Poll::Pending => false,
            };
            match self.feed(active, cx) {
                Err(e) => {
                    self.failed = true;
                    return Poll::Ready(Some(Err(e)));
                }
                Ok(fed) if read || fed => continue,
                Ok(_) => return Poll::Pending,
            }
        }
    }

    /// One frame of what is ready, within the credit; `end` when the input
    /// has ended. `Ok(true)`: something went out, so look again.
    fn feed(&mut self, active: &Active, cx: &mut Context<'_>) -> Result<bool, TransportError> {
        let Some(subscription) = active.guard.slot.subscription() else { return Ok(false) };
        let Some(input) = &mut self.input else { return Ok(false) };
        let mut items = String::new();
        let mut count = 0;
        let mut over = false;
        while self.credit > 0 && count < FRAME_ITEMS && items.len() < FRAME_BYTES {
            match Pin::new(&mut *input).poll_next(cx) {
                Poll::Ready(Some(Ok(item))) => {
                    if count > 0 {
                        items.push(',');
                    }
                    items.push_str(&item.to_string());
                    count += 1;
                    self.credit -= 1;
                }
                Poll::Ready(Some(Err(e))) => return Err(e),
                Poll::Ready(None) => {
                    over = true;
                    break;
                }
                Poll::Pending => break,
            }
        }
        // With no credit the input is not polled: its end is found once there
        // is, as the callee's half-window comes back or the call ends.
        let ns = self.registry.hub_namespace();
        if count > 0 {
            let frame = input_frame(ns, &subscription, self.seq, &items);
            trace("→", &frame);
            self.conn.send(frame)?;
            self.seq += 1;
        }
        if over {
            self.input = None;
            let frame = end_frame(ns, &subscription, self.seq);
            trace("→", &frame);
            self.conn.send(frame)?;
        }
        Ok(count > 0 || over)
    }
}

/// One call's life: the subscribe is on the wire, then the subscription is
/// drained.
enum CallState {
    Awaiting(Awaiting),
    Active(Active),
    Ended,
}

/// Subscribe written, reply outstanding.
struct Awaiting {
    reply: futures_channel::oneshot::Receiver<Result<Items, TransportError>>,
    guard: SubGuard,
}

impl Awaiting {
    async fn established(self) -> Result<Active, TransportError> {
        match self.reply.await {
            Ok(Ok(items)) => Ok(Active {
                items,
                guard: self.guard,
            }),
            Ok(Err(e)) => Err(e),
            // The reader dropped the waiter without answering, which only
            // happens if the connection died between the two.
            Err(_) => Err(TransportError::closed(
                "connection closed before the subscription was acknowledged",
            )),
        }
    }
}

/// Acknowledged; items are arriving.
struct Active {
    guard: SubGuard,
    items: Items,
}

impl Active {
    async fn next_item(&mut self) -> Option<Result<StreamItem, TransportError>> {
        loop {
            match self.items.next().await {
                // `done` / `error` end the subscription server-side, so there
                // is nothing left to unsubscribe from.
                Some(Inbound::Item(Ok(item))) => {
                    if item.is_terminal() {
                        self.guard.slot.mark_finished();
                    }
                    return Some(Ok(item));
                }
                // A transport failure does NOT mark the slot finished: if the
                // socket survived (a malformed item, say) the subscription is
                // still live and the guard must still cancel it. On a dead
                // socket the unsubscribe write simply fails and is ignored.
                Some(Inbound::Item(Err(e))) => return Some(Err(e)),
                // A plain call of a method with an input half: nothing to feed.
                Some(Inbound::Input(_)) => continue,
                None => return None,
            }
        }
    }
}

/// Cancels the server-side subscription when the caller drops the stream.
///
/// This is the piece `menger/web/index.html` is missing — it never
/// unsubscribes, and leaks a subscription per reload.
struct SubGuard {
    conn: backend::Conn,
    registry: Arc<Registry>,
    slot: Arc<SubSlot>,
}

impl Drop for SubGuard {
    fn drop(&mut self) {
        // `None` is either "nothing to cancel" or "the reply has not landed
        // yet" — in the second case the reader sends the unsubscribe when it
        // learns the id. See `SubSlot`.
        let Some(subscription) = self.slot.on_dropped() else {
            return;
        };
        self.registry.forget_sub(&subscription);
        let frame = self.registry.unsubscribe(&subscription);
        trace("→", &frame);
        // Dropping a stream is not a fallible operation for the caller, and
        // a failed unsubscribe means the socket is already gone — which
        // cancels the subscription anyway.
        let _ = self.conn.send(frame);
    }
}
