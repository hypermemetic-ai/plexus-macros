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
#[cfg(feature = "ws-native")]
mod served;
#[cfg(feature = "ws-native")]
pub use served::ServedStats;

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
use crate::transport::{ItemStream, MethodRef, Transport};
use futures_util::{stream, StreamExt};
use registry::{subscribe_frame_with, Items, Registry, SubSlot};
use serde_json::Value;
use std::sync::Arc;

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
    conn: Wire,
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
    conn: Wire,
}

/// LOCAL-3: how a frame is written — this transport's own socket, or a
/// writer handed in by whoever owns the socket (a server writing on a
/// connection it accepted: [`WsTransport::over`]).
pub type Writer = Arc<dyn Fn(String) -> Result<(), TransportError> + Send + Sync>;

#[derive(Clone)]
enum Wire {
    Socket(backend::Conn),
    Writer(Writer),
}

impl std::fmt::Debug for Wire {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Wire::Socket(c) => c.fmt(f),
            Wire::Writer(_) => f.write_str("Writer"),
        }
    }
}

impl Wire {
    fn send(&self, text: String) -> Result<(), TransportError> {
        match self {
            Wire::Socket(c) => c.send(text),
            Wire::Writer(w) => w(text),
        }
    }
    fn close(&self) {
        if let Wire::Socket(c) = self {
            c.close();
        }
    }
    #[cfg(feature = "ws-native")]
    fn writer(&self) -> Writer {
        let w = self.clone();
        Arc::new(move |t| w.send(t))
    }
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
        let conn = Wire::Socket(backend::connect(url, registry.clone()).await?);
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
        let conn = Wire::Socket(backend::connect_with_headers(url, headers, registry.clone()).await?);
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
        let call = format!("{}.call", self.registry.hub_namespace());
        self.subscribe_with(&call, &method.qualified(), params, None)
    }

    fn subscribe_with(
        &self,
        subscribe_method: &str,
        qualified: &str,
        params: Value,
        hop: Option<&Value>,
    ) -> Result<Awaiting, TransportError> {
        let request_id = self.registry.next_request_id();
        let slot = SubSlot::new();
        let reply = self.registry.register_pending(request_id, slot.clone())?;

        let frame = subscribe_frame_with(request_id, subscribe_method, qualified, &params, hop);
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
        drain(self.subscribe(method, params))
    }
}

/// LOCAL-3: the symmetric connection — either side opens a call on it.
impl WsTransport {
    /// A transport that writes through `write` instead of a socket of its
    /// own: a service calling into a peer on a connection it **accepted**.
    /// `peer_namespace` is the namespace the peer offered. Every inbound
    /// frame of that connection is handed to [`Self::take_frame`] first.
    /// The call table, the frames and the cancellation are this type's own —
    /// one implementation for both directions.
    pub fn over(peer_namespace: &str, write: Writer) -> Self {
        let registry = Arc::new(Registry::new(Arc::from(peer_namespace)));
        let conn = Wire::Writer(write);
        Self { _closer: Arc::new(Closer { conn: conn.clone() }), conn, registry }
    }

    /// Route one inbound frame of the connection [`Self::over`] writes on.
    /// `true` when it was this transport's (a response to its request, an
    /// item of a call it opened); `false` leaves it to the server.
    pub fn take_frame(&self, bytes: &[u8]) -> bool {
        // Fast path: a frame that is neither a response nor an item is
        // never ours (an ordinary call, an input frame) — skip the parse.
        if !contains(bytes, b"\"result\"") && !contains(bytes, b"\"error\"") {
            return false;
        }
        let Ok(frame) = serde_json::from_slice::<Value>(bytes) else { return false };
        if frame.get("method").is_some() && frame.get("id").is_some() {
            return false; // a request: the server's to dispatch
        }
        let routed = self.registry.route(&frame);
        if let Some(back) = routed.write_back {
            let _ = self.conn.send(back);
        }
        routed.taken
    }

    /// The connection [`Self::over`] writes on is gone: every call open on
    /// it ends with one `closed` transport error.
    pub fn on_close(&self, reason: &str) {
        self.registry.on_close(reason.to_string());
    }

    /// Open a call with `hop` (spec/12) on the frame: whom this side acts for.
    /// `method` is the dotted path (`ci.run`).
    pub fn call_with_hop(&self, method: &str, params: Value, hop: Option<Value>) -> ItemStream<'_> {
        let call = format!("{}.call", self.registry.hub_namespace());
        drain(self.subscribe_with(&call, method, params, hop.as_ref()))
    }

    /// Open a call whose input half this side will feed: the items stream,
    /// and a handle that sends `{ns}.input` / `{ns}.end` on it.
    /// `subscribe_method` is the method that opens it (`{ns}.call`, or a
    /// BIDIR-3 spike method).
    pub fn open_duplex(&self, subscribe_method: &str, method: &str, params: Value, input_ns: &str) -> (ItemStream<'_>, DuplexHandle) {
        let awaiting = self.subscribe_with(subscribe_method, method, params, None);
        let slot = awaiting.as_ref().ok().map(|a| a.guard.slot.clone());
        let handle = DuplexHandle { conn: self.conn.clone(), slot, input_ns: input_ns.to_string() };
        (drain(awaiting), handle)
    }

    /// What this side's call table holds: (requests awaiting a reply, live
    /// subscriptions). Both 0 once everything ended.
    pub fn held(&self) -> (usize, usize) {
        self.registry.held()
    }

    /// Send a plain request (not a subscribe) and await its `result`.
    pub async fn request(&self, method: &str, params: Value) -> Result<Value, TransportError> {
        let id = self.registry.next_request_id();
        let reply = self.registry.register_plain(id)?;
        let frame = serde_json::json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        trace("→", &frame);
        self.conn.send(frame)?;
        reply.await.unwrap_or_else(|_| Err(TransportError::closed("connection closed before the reply")))
    }
}

#[cfg(feature = "ws-native")]
impl WsTransport {
    /// Offer `surface` on this connection: from now on the far side may open
    /// calls into it (`{ns}.call` with `ns` = the surface's namespace), each
    /// answered by `surface` — which decides with its own gate. Sends
    /// `{service}.offer {namespace, activations, schema_hash, schema}` and
    /// returns the service's answer. Call it again after a reconnect: an
    /// offer belongs to one connection.
    pub async fn offer(&self, surface: Arc<dyn crate::offer::Offered>, window: u64) -> Result<Value, TransportError> {
        let served = served::Served::new(surface.clone(), self.conn.writer(), window);
        *self.registry.served.lock().unwrap_or_else(|e| e.into_inner()) = Some(served);
        let schema = surface.schema();
        let schema_text = schema.to_string();
        let params = serde_json::json!({
            "namespace": surface.namespace(),
            "activations": surface.activations(),
            "schema_hash": format!("{:016x}", fnv1a(schema_text.as_bytes())),
            "schema": schema,
        });
        let method = format!("{}.offer", self.registry.hub_namespace());
        self.request(&method, params).await
    }

    /// End every call this side serves (`offer_withdrawn`), from this side.
    pub fn withdraw_offer(&self) {
        if let Some(s) = self.registry.served() {
            s.withdraw();
        }
    }

    /// The served calls' counts, once offered.
    pub fn served_stats(&self) -> Option<(usize, u64, u64, u64)> {
        self.registry.served().map(|s| {
            use std::sync::atomic::Ordering::Relaxed;
            (s.open_calls(), s.stats.opened.load(Relaxed), s.stats.not_offered.load(Relaxed), s.stats.cancelled.load(Relaxed))
        })
    }
}

fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// LOCAL-3: feeds a call's input half — the same `{ns}.input` / `{ns}.end`
/// frames whichever side opened the call.
#[derive(Debug, Clone)]
pub struct DuplexHandle {
    conn: Wire,
    slot: Option<Arc<SubSlot>>,
    input_ns: String,
}

impl DuplexHandle {
    /// The call's subscription id, once acknowledged.
    pub fn subscription(&self) -> Option<Value> {
        self.slot.as_ref().and_then(|s| s.subscription())
    }
    /// `{ns}.input {subscription, seq, items}`.
    pub fn input(&self, seq: u64, items: Vec<Value>) -> Result<(), TransportError> {
        let sub = self.subscription().ok_or_else(|| TransportError::protocol("not acknowledged yet"))?;
        self.conn.send(
            serde_json::json!({"jsonrpc": "2.0", "method": format!("{}.input", self.input_ns), "params": {"subscription": sub, "seq": seq, "items": items}})
                .to_string(),
        )
    }
    /// `{ns}.end {subscription, seq}` — `seq` the number of input frames sent.
    pub fn end(&self, seq: u64) -> Result<(), TransportError> {
        let sub = self.subscription().ok_or_else(|| TransportError::protocol("not acknowledged yet"))?;
        self.conn.send(
            serde_json::json!({"jsonrpc": "2.0", "method": format!("{}.end", self.input_ns), "params": {"subscription": sub, "seq": seq}})
                .to_string(),
        )
    }
}

fn drain<'a>(awaiting: Result<Awaiting, TransportError>) -> ItemStream<'a> {
    {
        let start = match awaiting {
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
        match self.items.next().await {
            // `done` / `error` end the subscription server-side, so there is
            // nothing left to unsubscribe from.
            Some(Ok(item)) => {
                if item.is_terminal() {
                    self.guard.slot.mark_finished();
                }
                Some(Ok(item))
            }
            // A transport failure does NOT mark the slot finished: if the
            // socket survived (a malformed item, say) the subscription is
            // still live and the guard must still cancel it. On a dead
            // socket the unsubscribe write simply fails and is ignored.
            Some(Err(e)) => Some(Err(e)),
            None => None,
        }
    }
}

/// Cancels the server-side subscription when the caller drops the stream.
///
/// This is the piece `menger/web/index.html` is missing — it never
/// unsubscribes, and leaks a subscription per reload.
struct SubGuard {
    conn: Wire,
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
