//! The backend-independent half of the WebSocket transport: request ids,
//! the pending-subscribe table, the subscription routing table, and the
//! two frame shapes.
//!
//! Everything here is pure state plus `serde_json` — no socket, no runtime.
//! That is what lets the native (`tokio-tungstenite`) and browser
//! (`web_sys::WebSocket`) backends share one protocol implementation
//! instead of two that drift.

use crate::envelope::{Content, RawItem, StreamItem, StreamMetadata};
use crate::error::TransportError;
use futures_channel::{mpsc, oneshot};
use serde::de::{Error as _, IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};
use serde_json::value::RawValue;
use serde_json::{json, Value};
use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// An item as a call reads it: a [`StreamItem`] for `call`, a [`RawItem`]
/// for `call_raw`, a typed client's (PERF.115-36). Which one is the call's
/// own, fixed when it subscribes.
#[derive(Debug)]
pub(crate) enum Delivered {
    Item(StreamItem),
    Raw(RawItem),
}

impl Delivered {
    pub(crate) fn is_terminal(&self) -> bool {
        match self {
            Delivered::Item(item) => item.is_terminal(),
            Delivered::Raw(item) => item.is_terminal(),
        }
    }

    /// The item a `call` reads. A call reads one kind only.
    pub(crate) fn into_item(self) -> StreamItem {
        match self {
            Delivered::Item(item) => item,
            Delivered::Raw(_) => unreachable!("a call is handed the items it asked for"),
        }
    }

    /// The item a `call_raw` reads.
    pub(crate) fn into_raw(self) -> RawItem {
        match self {
            Delivered::Raw(item) => item,
            Delivered::Item(item) => RawItem::from(item),
        }
    }
}

/// The receiving half of one subscription's item channel.
pub(crate) type Items = mpsc::UnboundedReceiver<Result<Delivered, TransportError>>;
type ItemsTx = mpsc::UnboundedSender<Result<Delivered, TransportError>>;

/// One call's cancellation state, shared between the caller's stream and the
/// connection's reader.
///
/// It exists because of one ordering problem. A subscribe is written the
/// moment [`Transport::call`](crate::Transport::call) is invoked — it has to
/// be, or a `watch` "subscribed before the mutation" would only actually
/// subscribe when someone first polled it, and the mutation would be missed.
/// But the subscription *id* only arrives with the reply, and the caller may
/// drop the stream before then. Whoever learns last is the one that has to
/// cancel:
///
/// | Order | Who sends `call_unsub` |
/// |---|---|
/// | reply, then drop | the stream's guard — it can read the id from here |
/// | drop, then reply | the reader — [`Registry::on_text`] hands it the frame |
///
/// Both read and write this under one lock, so exactly one of them fires.
#[derive(Debug, Default)]
pub(crate) struct SubSlot {
    state: Mutex<SlotState>,
}

#[derive(Debug, Default)]
struct SlotState {
    /// The subscription id, once the server has acknowledged.
    subscription: Option<Value>,
    /// The caller dropped its stream.
    dropped: bool,
    /// A terminal item arrived: the server already ended the subscription,
    /// so cancelling it would only earn an "unknown subscription" error.
    finished: bool,
}

impl SubSlot {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// The stream saw `done` or `error`.
    pub(crate) fn mark_finished(&self) {
        self.lock().finished = true;
    }

    /// The caller dropped the stream. `Some(id)` means "cancel it yourself,
    /// here is the id"; `None` means either nothing to cancel or the reply
    /// has not landed yet, in which case the reader will do it.
    pub(crate) fn on_dropped(&self) -> Option<Value> {
        let mut state = self.lock();
        state.dropped = true;
        if state.finished {
            return None;
        }
        state.subscription.clone()
    }

    /// The reply landed. Returns `true` if the caller is already gone and
    /// the reader must cancel the subscription it just learned about.
    fn acknowledged(&self, subscription: &Value) -> bool {
        let mut state = self.lock();
        state.subscription = Some(subscription.clone());
        state.dropped && !state.finished
    }

    fn lock(&self) -> MutexGuard<'_, SlotState> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// A subscribe that has been written and is waiting for its reply.
#[derive(Debug)]
struct Pending {
    slot: Arc<SubSlot>,
    waiter: oneshot::Sender<Result<Items, TransportError>>,
    /// The call reads [`RawItem`]s (a typed client).
    raw: bool,
}

/// A live subscription: where its items go, and its cancellation state.
///
/// The slot is kept here as well as in the caller's guard so that a
/// *terminal item* can mark it finished. The server ending a subscription is
/// a fact the connection knows even when the caller has not read the `done`
/// yet — and without it, dropping a half-read unary call sends a
/// `call_unsub` the server answers `false`.
#[derive(Debug)]
struct Sub {
    sink: ItemsTx,
    slot: Arc<SubSlot>,
    /// The call reads [`RawItem`]s (a typed client).
    raw: bool,
    /// The content type its last data item named, shared with the next that
    /// names the same: every item of a call does.
    content_type: Option<Arc<str>>,
}

#[derive(Debug, Default)]
struct State {
    /// JSON-RPC request id → the call waiting for that subscribe's reply.
    pending: HashMap<u64, Pending>,
    /// subscription key → that subscription's sink and cancellation state.
    subs: HashMap<String, Sub>,
    /// Set once, when the socket goes away.
    closed: Option<TransportError>,
    /// Notifications for an id no subscription has yet, kept while a call
    /// waits for its id (spec/02 rule 4), in the order they came.
    early: Vec<(String, String)>,
    /// Their bytes.
    early_bytes: usize,
}

/// How much is kept for ids not yet answered: a server keeps rule 1 and sends
/// none, so this only has to be enough for one that does not.
const EARLY_FRAMES: usize = 1024;
const EARLY_BYTES: usize = 4 << 20;

/// A notification for an id no subscription has: a call still waiting for its
/// reply may be about to learn it (spec/02 rule 4), so it is kept while one
/// waits, within bounds. With none waiting it is a call that is over.
fn keep_early(state: &mut State, key: &str, text: &str) {
    if !state.pending.is_empty() && state.early.len() < EARLY_FRAMES && state.early_bytes + text.len() <= EARLY_BYTES {
        state.early_bytes += text.len();
        state.early.push((key.to_owned(), text.to_owned()));
    }
}

/// One connection's protocol state.
#[derive(Debug)]
pub(crate) struct Registry {
    hub_namespace: Arc<str>,
    next_id: AtomicU64,
    state: Mutex<State>,
}

/// The item `type`s this client reads (spec/03); any other is skipped.
const KINDS: [&str; 5] = ["data", "progress", "error", "request", "done"];

/// The routing key for a subscription id.
///
/// Rendered as JSON text rather than "the string if it is one", so a server
/// that answers `7` and a server that answers `"7"` cannot collide.
fn key_of(id: &Value) -> String {
    id.to_string()
}

/// [`key_of`] for an id still as its text: the text itself where it is what
/// `key_of` writes (an integer of at most 19 digits, a string with nothing
/// escaped), so the two readings route alike; anything else through a `Value`.
fn key_of_text(id: &RawValue) -> Cow<'_, str> {
    let text = id.get();
    let bytes = text.as_bytes();
    let integer = !bytes.is_empty() && bytes.len() <= 19 && bytes.iter().all(u8::is_ascii_digit) && (bytes.len() == 1 || bytes[0] != b'0');
    let string = bytes.len() >= 2
        && bytes[0] == b'"'
        && bytes[bytes.len() - 1] == b'"'
        && !bytes[1..bytes.len() - 1].iter().any(|&b| b == b'\\' || b == b'"' || b < 0x20);
    if integer || string {
        Cow::Borrowed(text)
    } else {
        Cow::Owned(key_of(&value_of(id)))
    }
}

/// A piece of a frame, as a `Value`.
fn value_of(text: &RawValue) -> Value {
    serde_json::from_str(text.get()).unwrap_or(Value::Null)
}

/// A frame as the reader reads it (PERF.115-36): nothing built of it. A
/// reply's `id`, `result` and `error` stay their JSON text, and a
/// notification's subscription and item are read in place. A frame that does
/// not read this way is read as a `Value`, as before ([`Registry::on_value`]).
#[derive(Deserialize)]
struct Frame<'a> {
    #[serde(default, borrow)]
    id: Option<&'a RawValue>,
    #[serde(default, borrow)]
    result: Option<&'a RawValue>,
    #[serde(default, borrow)]
    error: Option<&'a RawValue>,
    #[serde(default, borrow)]
    params: Option<Note<'a>>,
}

/// A notification's `params`, an object: whose item it is, and the item.
struct Note<'a> {
    subscription: &'a RawValue,
    item: Fields<'a>,
}

impl<'de: 'a, 'a> Deserialize<'de> for Note<'a> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct Read<'a>(std::marker::PhantomData<&'a ()>);
        impl<'de: 'a, 'a> Visitor<'de> for Read<'a> {
            type Value = Note<'a>;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a notification's params")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Note<'a>, A::Error> {
                let (mut subscription, mut item) = (None, None);
                while let Some(key) = map.next_key::<Cow<'de, str>>()? {
                    match key.as_ref() {
                        "subscription" => subscription = Some(map.next_value()?),
                        "result" => item = Some(map.next_value()?),
                        _ => drop(map.next_value::<IgnoredAny>()?),
                    }
                }
                Ok(Note {
                    subscription: subscription.ok_or_else(|| A::Error::missing_field("subscription"))?,
                    item: item.ok_or_else(|| A::Error::missing_field("result"))?,
                })
            }
        }
        d.deserialize_map(Read(std::marker::PhantomData))
    }
}

/// An item's fields, read in place: its content, metadata and request data
/// left as their text, what a typed decode never reads never built.
#[derive(Deserialize)]
struct Fields<'a> {
    #[serde(rename = "type")]
    kind: Option<Kind>,
    #[serde(default, borrow)]
    metadata: Option<&'a RawValue>,
    #[serde(default, borrow)]
    content_type: Option<Cow<'a, str>>,
    #[serde(default, borrow, deserialize_with = "present")]
    content: Option<&'a RawValue>,
    #[serde(default, borrow)]
    message: Option<Cow<'a, str>>,
    #[serde(default)]
    percentage: Option<f32>,
    #[serde(default, borrow)]
    code: Option<Cow<'a, str>>,
    #[serde(default)]
    recoverable: Option<bool>,
    #[serde(default, borrow, rename = "requestId")]
    request_id: Option<Cow<'a, str>>,
    #[serde(default, borrow, rename = "requestData", deserialize_with = "present")]
    request_data: Option<&'a RawValue>,
    #[serde(default, rename = "timeoutMs")]
    timeout_ms: Option<u64>,
}

/// A field that is there, as its text, `null` included: a `Value` field reads
/// `null` as itself, where an `Option` would read it as absent.
fn present<'de: 'a, 'a, D: Deserializer<'de>>(d: D) -> Result<Option<&'a RawValue>, D::Error> {
    <&'a RawValue>::deserialize(d).map(Some)
}

/// An item's `type`: the five this client reads, or one it skips (spec/03).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    Data,
    Progress,
    Error,
    Request,
    Done,
    #[serde(other)]
    Other,
}

impl Fields<'_> {
    /// The [`StreamItem`] serde makes of the item, made from its fields; `None`
    /// where they make none, for the `Value` reading to say why.
    fn stream_item(&self, kind: Kind) -> Option<StreamItem> {
        let metadata = match self.metadata {
            Some(m) if m.get() != "null" => Some(serde_json::from_str::<StreamMetadata>(m.get()).ok()?),
            _ => None,
        };
        Some(match kind {
            Kind::Data => StreamItem::Data {
                metadata,
                content_type: self.content_type.as_deref().map(str::to_owned),
                content: serde_json::from_str(self.content?.get()).ok()?,
            },
            Kind::Progress => StreamItem::Progress {
                metadata,
                message: self.message.as_deref()?.to_owned(),
                percentage: self.percentage,
            },
            Kind::Error => StreamItem::Error {
                metadata,
                message: self.message.as_deref()?.to_owned(),
                code: self.code.as_deref().map(str::to_owned),
                recoverable: self.recoverable.unwrap_or(false),
            },
            Kind::Request => StreamItem::Request {
                request_id: self.request_id.as_deref()?.to_owned(),
                request_data: serde_json::from_str(self.request_data?.get()).ok()?,
                timeout_ms: self.timeout_ms.unwrap_or(0),
            },
            Kind::Done => StreamItem::Done { metadata },
            Kind::Other => return None,
        })
    }

    /// The [`RawItem`] a typed call reads: its content the item's own text,
    /// its content type `content_type`.
    fn raw_item(&self, kind: Kind, content_type: Option<Arc<str>>) -> Option<RawItem> {
        Some(match kind {
            Kind::Data => RawItem::Data { content_type, content: Content::from(self.content?.to_owned()) },
            Kind::Progress => RawItem::Progress { message: self.message.as_deref()?.to_owned(), percentage: self.percentage },
            Kind::Error => RawItem::Error {
                message: self.message.as_deref()?.to_owned(),
                code: self.code.as_deref().map(str::to_owned),
                recoverable: self.recoverable.unwrap_or(false),
            },
            Kind::Request => RawItem::Request,
            Kind::Done => RawItem::Done,
            Kind::Other => return None,
        })
    }
}

impl Registry {
    pub(crate) fn new(hub_namespace: Arc<str>) -> Self {
        Self {
            hub_namespace,
            // JSON-RPC ids start at 1; 0 reads like "unset" in a trace.
            next_id: AtomicU64::new(1),
            state: Mutex::new(State::default()),
        }
    }

    pub(crate) fn next_request_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    pub(crate) fn hub_namespace(&self) -> &str {
        &self.hub_namespace
    }

    /// Claim a request id for a subscribe about to be written.
    ///
    /// Fails immediately if the socket is already gone, so a call on a dead
    /// connection reports `Closed` instead of hanging on a reply that will
    /// never come.
    pub(crate) fn register_pending(
        &self,
        request_id: u64,
        slot: Arc<SubSlot>,
        raw: bool,
    ) -> Result<oneshot::Receiver<Result<Items, TransportError>>, TransportError> {
        let mut state = self.lock();
        if let Some(err) = &state.closed {
            return Err(err.clone());
        }
        let (waiter, reply) = oneshot::channel();
        state.pending.insert(request_id, Pending { slot, waiter, raw });
        Ok(reply)
    }

    /// Give up on a subscribe whose frame could not be written.
    pub(crate) fn forget_pending(&self, request_id: u64) {
        self.lock().pending.remove(&request_id);
    }

    /// Drop a subscription's routing entry (the stream was dropped, or the
    /// server sent its terminal item).
    pub(crate) fn forget_sub(&self, subscription: &Value) {
        self.forget_key(&key_of(subscription));
    }

    fn forget_key(&self, key: &str) {
        self.lock().subs.remove(key);
    }

    /// Route one inbound text frame, returning a frame the connection should
    /// write back (only ever an unsubscribe for a call whose caller vanished
    /// before the acknowledgement).
    ///
    /// Three frame shapes matter, and exactly one thing decides between
    /// them — never the method name:
    ///
    /// | Frame | Recognised by | Routed to |
    /// |---|---|---|
    /// | subscribe reply | an `id` plus `result`/`error` | the pending entry for that id |
    /// | stream item | `params.subscription` | that subscription's channel |
    /// | anything else | — | ignored |
    ///
    /// The second row is the whole reason this transport is hand-written.
    /// `plexus_core::plexus::PLEXUS_NOTIF_METHOD` is `"result"`, but
    /// plexus-core 0.5.3 emits notifications under the *subscribe method's*
    /// own name — `"menger.call"` — which is why `menger/web/index.html`
    /// accepts both. Keying on `params.subscription` makes the method name
    /// irrelevant, so neither spelling can break routing.
    #[must_use = "the returned frame cancels a subscription nobody is reading"]
    ///
    /// The frame is read in place ([`Frame`], PERF.115-36): its item's
    /// content stays text until the call reads it, a typed call's straight
    /// into its event. What does not read so is read as a `Value`, as before.
    pub(crate) fn on_text(&self, text: &str) -> Option<String> {
        let Ok(frame) = serde_json::from_str::<Frame<'_>>(text) else {
            return self.on_value(text);
        };

        // A subscribe reply.
        if let Some(request_id) = frame.id.and_then(|id| id.get().parse::<u64>().ok()) {
            if frame.result.is_some() || frame.error.is_some() {
                return self.on_response(request_id, frame.result.map(value_of), frame.error.map(value_of));
            }
        }

        // A stream item.
        if let Some(note) = frame.params {
            self.on_note(note, text);
        }
        None
    }

    /// [`on_text`](Self::on_text) through a `Value`: for a frame the reading
    /// in place does not take, which this reads as it always has.
    fn on_value(&self, text: &str) -> Option<String> {
        let frame = serde_json::from_str::<Value>(text).ok()?;

        // A subscribe reply.
        if let Some(request_id) = frame.get("id").and_then(Value::as_u64) {
            if frame.get("result").is_some() || frame.get("error").is_some() {
                return self.on_response(request_id, frame.get("result").cloned(), frame.get("error").cloned());
            }
        }

        // A stream item.
        if let Some(subscription) = frame.pointer("/params/subscription") {
            self.on_notification(&subscription.clone(), &frame, text);
        }
        None
    }

    fn on_response(&self, request_id: u64, result: Option<Value>, error: Option<Value>) -> Option<String> {
        let mut state = self.lock();
        // Not ours: the reply to an unsubscribe, or a duplicate.
        let pending = state.pending.remove(&request_id)?;

        if let Some(error) = error {
            if state.pending.is_empty() {
                state.early.clear();
                state.early_bytes = 0;
            }
            drop(state);
            let _ = pending.waiter.send(Err(TransportError::protocol(format!(
                "subscribe rejected: {error}"
            ))));
            return None;
        }

        let subscription = result.unwrap_or(Value::Null);

        // The caller dropped its stream before the acknowledgement arrived,
        // so it never learned this id and could not cancel it. Cancel it
        // here instead of registering a channel nobody will read.
        if pending.slot.acknowledged(&subscription) {
            drop(state);
            drop(self.take_early(&key_of(&subscription)));
            return Some(self.unsubscribe(&subscription));
        }

        let key = key_of(&subscription);
        let (sink, items) = mpsc::unbounded();
        state.subs.insert(
            key.clone(),
            Sub {
                sink,
                slot: pending.slot.clone(),
                raw: pending.raw,
                content_type: None,
            },
        );
        drop(state);

        if pending.waiter.send(Ok(items)).is_err() {
            // Dropped in the instant between `acknowledged` and here; the
            // stream's own guard has the id and is cancelling it. Just stop
            // routing to a receiver that is gone.
            self.lock().subs.remove(&key);
        }
        // What came for this id before its reply goes first, in order: the
        // reader is the only caller of `on_text`, so nothing overtakes it.
        for frame in self.take_early(&key) {
            let _ = self.on_text(&frame);
        }
        None
    }

    /// The notifications kept for `key`; and, once no call waits for an id,
    /// everything kept, which belongs to calls that are over.
    fn take_early(&self, key: &str) -> Vec<String> {
        let mut state = self.lock();
        if state.early.is_empty() {
            return Vec::new();
        }
        let (mine, rest): (Vec<_>, Vec<_>) = std::mem::take(&mut state.early).into_iter().partition(|(k, _)| k == key);
        if !state.pending.is_empty() {
            state.early = rest;
        }
        state.early_bytes = state.early.iter().map(|(_, f)| f.len()).sum();
        mine.into_iter().map(|(_, f)| f).collect()
    }

    /// A stream item read in place: the call's kind of item made of its
    /// fields, or, where they make none, the frame read as a `Value`, which
    /// says why as it always has.
    fn on_note(&self, note: Note<'_>, text: &str) {
        let Some(kind) = note.item.kind else {
            return drop(self.on_value(text));
        };
        // spec/03: an item of a `type` this client does not know is skipped.
        if kind == Kind::Other {
            return;
        }
        let key = key_of_text(note.subscription);
        let mut state = self.lock();
        let Some(sub) = state.subs.get_mut(key.as_ref()) else {
            keep_early(&mut state, &key, text);
            return;
        };
        let (sink, slot) = (sub.sink.clone(), sub.slot.clone());
        let item = if sub.raw {
            let content_type = match note.item.content_type.as_deref() {
                Some(named) if sub.content_type.as_deref() == Some(named) => sub.content_type.clone(),
                Some(named) => {
                    let shared: Arc<str> = Arc::from(named);
                    sub.content_type = Some(shared.clone());
                    Some(shared)
                }
                None => None,
            };
            drop(state);
            note.item.raw_item(kind, content_type).map(Delivered::Raw)
        } else {
            drop(state);
            note.item.stream_item(kind).map(Delivered::Item)
        };
        let Some(item) = item else {
            return drop(self.on_value(text));
        };
        let terminal = item.is_terminal();
        let _ = sink.unbounded_send(Ok(item));
        if terminal {
            // The server ended it; there is nothing left to cancel,
            // whether or not the caller ever reads this item.
            slot.mark_finished();
            self.forget_key(&key);
        }
    }

    fn on_notification(&self, subscription: &Value, frame: &Value, text: &str) {
        let key = key_of(subscription);
        let mut state = self.lock();
        let Some((sink, slot, raw)) = state
            .subs
            .get(&key)
            .map(|sub| (sub.sink.clone(), sub.slot.clone(), sub.raw))
        else {
            keep_early(&mut state, &key, text);
            return;
        };
        drop(state);

        let payload = frame.pointer("/params/result").cloned().unwrap_or(Value::Null);
        // spec/03: an item of a `type` this client does not know is not
        // terminal; it is skipped, and the stream goes on.
        if payload.get("type").and_then(Value::as_str).is_some_and(|kind| !KINDS.contains(&kind)) {
            return;
        }
        match serde_json::from_value::<StreamItem>(payload) {
            Ok(item) => {
                let terminal = item.is_terminal();
                let item = if raw { Delivered::Raw(RawItem::from(item)) } else { Delivered::Item(item) };
                let _ = sink.unbounded_send(Ok(item));
                if terminal {
                    // The server ended it; there is nothing left to cancel,
                    // whether or not the caller ever reads this item.
                    slot.mark_finished();
                    self.forget_sub(subscription);
                }
            }
            Err(e) => {
                let _ = sink.unbounded_send(Err(TransportError::protocol(format!(
                    "notification payload is not a Plexus stream item: {e}"
                ))));
                self.forget_sub(subscription);
            }
        }
    }

    /// The socket is gone. Every waiting subscribe and every live stream
    /// learns about it as a [`TransportErrorKind::Closed`] item, which is
    /// what makes a mid-stream disconnect visible rather than a stream that
    /// merely stops.
    ///
    /// [`TransportErrorKind::Closed`]: crate::TransportErrorKind::Closed
    pub(crate) fn on_close(&self, reason: impl Into<String>) {
        let error = TransportError::closed(reason);
        let (pending, subs) = {
            let mut state = self.lock();
            if state.closed.is_some() {
                return;
            }
            state.closed = Some(error.clone());
            state.early = Vec::new();
            state.early_bytes = 0;
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.subs),
            )
        };

        for (_, waiting) in pending {
            let _ = waiting.waiter.send(Err(error.clone()));
        }
        for (_, sub) in subs {
            let _ = sub.sink.unbounded_send(Err(error.clone()));
        }
    }

    pub(crate) fn is_closed(&self) -> bool {
        self.lock().closed.is_some()
    }

    /// The unsubscribe frame for a subscription, with a fresh request id.
    pub(crate) fn unsubscribe(&self, subscription: &Value) -> String {
        unsubscribe_frame(self.next_request_id(), &self.hub_namespace, subscription)
    }

    /// The lock is only ever held for map surgery — never across an await,
    /// and never while sending on a channel that could re-enter.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// The subscribe frame, verbatim as menger's `web/index.html` and
/// `tests/wire.rs` send it:
///
/// ```json
/// {"jsonrpc":"2.0","id":1,"method":"menger.call",
///  "params":{"method":"tickets.watch","params":{"epic":"MACRO"}}}
/// ```
///
/// The hub namespace (`menger`) is `DynamicHub::new(ns)` — a property of the
/// *server*, not of the activation — which is why it is a constructor
/// argument of the transport and not something [`MethodRef`] could supply.
///
/// [`MethodRef`]: crate::MethodRef
pub(crate) fn subscribe_frame(
    request_id: u64,
    hub_namespace: &str,
    qualified_method: &str,
    params: &Value,
) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": format!("{hub_namespace}.call"),
        "params": { "method": qualified_method, "params": params },
    })
    .to_string()
}

/// The unsubscribe frame: `<hub>.call_unsub` with the subscription id as the
/// single positional parameter — the shape jsonrpsee's server registers
/// alongside every subscription method.
///
/// `web/index.html` never sends one and leaks a subscription per reload;
/// dropping a stream from this transport sends it.
fn unsubscribe_frame(request_id: u64, hub_namespace: &str, subscription: &Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": format!("{hub_namespace}.call_unsub"),
        "params": [subscription],
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{FutureExt, StreamExt};

    /// A subscribe written as request `id` and answered with `subscription`:
    /// the channel its items arrive on.
    fn subscribed(registry: &Registry, id: u64, subscription: &str) -> Items {
        subscribed_as(registry, id, subscription, false)
    }

    /// The same, for a call that reads [`RawItem`]s (`raw`) or [`StreamItem`]s.
    fn subscribed_as(registry: &Registry, id: u64, subscription: &str, raw: bool) -> Items {
        let reply = registry.register_pending(id, SubSlot::new(), raw).unwrap();
        let answer = format!(r#"{{"jsonrpc":"2.0","id":{id},"result":{subscription}}}"#);
        assert!(registry.on_text(&answer).is_none());
        reply.now_or_never().expect("answered").expect("not dropped").expect("subscribed")
    }

    /// A notification for `subscription` carrying `item`.
    fn note(subscription: &str, item: &str) -> String {
        format!(r#"{{"jsonrpc":"2.0","method":"kit.call","params":{{"subscription":{subscription},"result":{item}}}}}"#)
    }

    const DATA: &str = r#"{"type":"data","metadata":{"provenance":["kit"],"plexus_hash":"h","timestamp":1},"content_type":"kit.say","content":{"type":"said","text":"hi"}}"#;
    const DONE: &str = r#"{"type":"done","metadata":{"provenance":["kit"],"plexus_hash":"h","timestamp":1}}"#;

    /// Everything the channel holds now.
    fn drain(items: &mut Items) -> Vec<Result<StreamItem, TransportError>> {
        std::iter::from_fn(|| items.next().now_or_never().flatten()).map(|item| item.map(Delivered::into_item)).collect()
    }

    /// Everything a typed call's channel holds now.
    fn drain_raw(items: &mut Items) -> Vec<Result<RawItem, TransportError>> {
        std::iter::from_fn(|| items.next().now_or_never().flatten()).map(|item| item.map(Delivered::into_raw)).collect()
    }

    /// spec/03: a client SHOULD treat an item `type` it does not know as
    /// non-terminal and skip it. PERF.115-41 found one ended the stream, so a
    /// new kind of item (a stream's header, say) would end every call that
    /// met it, and menger's `watch` would reconnect.
    #[test]
    fn an_item_of_a_type_it_does_not_know_is_skipped_and_the_stream_goes_on() {
        let registry = Registry::new(Arc::from("kit"));
        let mut items = subscribed(&registry, 1, "7");
        for item in [
            r#"{"type":"header","metadata":{"provenance":["kit"],"plexus_hash":"h","timestamp":1},"columns":["a"]}"#,
            r#"{"type":"guidance","error_type":"retired","suggestion":"none"}"#,
            DATA,
            DONE,
        ] {
            assert!(registry.on_text(&note("7", item)).is_none());
        }
        let got = drain(&mut items);
        assert!(matches!(got[..], [Ok(StreamItem::Data { .. }), Ok(StreamItem::Done { .. })]), "{got:?}");
    }

    /// spec/02 rule 4: a client SHOULD keep a notification for an id it has
    /// not yet seen the response for. The spec says plexus-client does;
    /// PERF.115-41 found it dropped it.
    #[test]
    fn an_item_before_its_subscription_id_is_kept_until_the_id_arrives() {
        let registry = Registry::new(Arc::from("kit"));
        let reply = registry.register_pending(1, SubSlot::new(), false).unwrap();
        assert!(registry.on_text(&note("7", DATA)).is_none());
        assert!(registry.on_text(r#"{"jsonrpc":"2.0","id":1,"result":7}"#).is_none());
        assert!(registry.on_text(&note("7", DONE)).is_none());
        let mut items = reply.now_or_never().unwrap().unwrap().unwrap();
        let got = drain(&mut items);
        assert!(matches!(got[..], [Ok(StreamItem::Data { .. }), Ok(StreamItem::Done { .. })]), "{got:?}");
    }

    /// Kept only while a call waits for its id: an item for an id no call is
    /// waiting on belongs to a call that is over, and a later call answered
    /// with that id does not get it.
    #[test]
    fn an_item_for_an_id_no_call_waits_on_is_dropped() {
        let registry = Registry::new(Arc::from("kit"));
        let _first = subscribed(&registry, 1, "7");
        assert!(registry.on_text(&note("8", DATA)).is_none());
        let mut items = subscribed(&registry, 2, "8");
        assert!(registry.on_text(&note("8", DONE)).is_none());
        let got = drain(&mut items);
        assert!(matches!(got[..], [Ok(StreamItem::Done { .. })]), "{got:?}");
    }

    /// PERF.115-36: a typed call is handed its data item's content as the
    /// wire's text, to decode straight into its event, and its content type.
    #[test]
    fn a_typed_call_is_handed_the_content_as_the_wires_text() {
        let registry = Registry::new(Arc::from("kit"));
        let mut items = subscribed_as(&registry, 1, "7", true);
        for item in [DATA, DONE] {
            assert!(registry.on_text(&note("7", item)).is_none());
        }
        match &drain_raw(&mut items)[..] {
            [Ok(RawItem::Data { content, content_type }), Ok(RawItem::Done)] => {
                assert_eq!(content.as_text(), Some(r#"{"type":"said","text":"hi"}"#));
                assert_eq!(content_type.as_deref(), Some("kit.say"));
            }
            other => panic!("{other:?}"),
        }
    }

    /// A typed call and a call that reads `StreamItem`s read the same items
    /// from the same frames.
    #[test]
    fn a_typed_call_and_a_value_call_read_the_same_items() {
        let registry = Registry::new(Arc::from("kit"));
        let mut values = subscribed_as(&registry, 1, "7", false);
        let mut raws = subscribed_as(&registry, 2, "\"s-8\"", true);
        let items = [
            DATA,
            r#"{"type":"data","content":{"n":[1,2.5,"x\"y"]},"content_type":"kit.n"}"#,
            r#"{"type":"progress","metadata":{"provenance":["kit"],"plexus_hash":"h","timestamp":1},"message":"half","percentage":50.0}"#,
            r#"{"type":"progress","message":"none yet"}"#,
            r#"{"type":"request","requestId":"r-1","requestData":{"type":"confirm"},"timeoutMs":30000}"#,
            r#"{"type":"error","metadata":{"provenance":["kit"],"plexus_hash":"h","timestamp":1},"message":"no","code":"forbidden","recoverable":true}"#,
        ];
        for item in items {
            assert!(registry.on_text(&note("7", item)).is_none());
            assert!(registry.on_text(&note("\"s-8\"", item)).is_none());
        }
        let (values, raws) = (drain(&mut values), drain_raw(&mut raws));
        assert_eq!(values.len(), items.len(), "{values:?}");
        assert_eq!(raws.len(), items.len(), "{raws:?}");
        for (value, raw) in values.into_iter().zip(raws) {
            match (value.unwrap(), raw.unwrap()) {
                (StreamItem::Data { content: v, content_type: a, .. }, RawItem::Data { content: r, content_type: b }) => {
                    assert_eq!(r.into_value().unwrap(), v);
                    assert_eq!(a.as_deref(), b.as_deref());
                }
                (StreamItem::Progress { message: m, percentage: p, .. }, RawItem::Progress { message, percentage }) => {
                    assert_eq!((m, p), (message, percentage));
                }
                (StreamItem::Request { .. }, RawItem::Request) => {}
                (StreamItem::Error { message: m, code: c, recoverable: r, .. }, RawItem::Error { message, code, recoverable }) => {
                    assert_eq!((m, c, r), (message, code, recoverable));
                }
                other => panic!("{other:?}"),
            }
        }
    }

    /// The in-place reading makes of each well-formed item the `StreamItem`
    /// serde makes of it (PERF.115-36).
    #[test]
    fn the_in_place_reading_makes_the_stream_item_serde_makes() {
        for item in [
            DATA,
            DONE,
            r#"{"type":"done"}"#,
            r#"{"type":"data","content":{"n":[1,2.5,"x\"y"]},"content_type":"kit.n"}"#,
            r#"{"type":"data","metadata":null,"content_type":null,"content":null}"#,
            r#"{"content":{"a":1},"extra":{"ignored":[true]},"type":"data"}"#,
            r#"{"type":"data","metadata":{"provenance":["a","b"],"plexus_hash":"x","timestamp":-1,"more":1},"content":"s"}"#,
            r#"{"type":"data","metadata":{},"content":{}}"#,
            r#"{"type":"progress","metadata":{"provenance":["kit"],"plexus_hash":"h","timestamp":1},"message":"half","percentage":50.0}"#,
            r#"{"type":"progress","message":"none yet","percentage":null}"#,
            r#"{"type":"request","requestId":"r-1","requestData":{"type":"confirm"},"timeoutMs":30000}"#,
            r#"{"type":"request","requestId":"r-2","requestData":null}"#,
            r#"{"type":"error","message":"no","code":"forbidden","recoverable":true,"metadata":{"provenance":[],"plexus_hash":"","timestamp":0}}"#,
            r#"{"type":"error","message":"no","code":null}"#,
        ] {
            let fields: Fields<'_> = serde_json::from_str(item).unwrap();
            let built = fields.stream_item(fields.kind.unwrap());
            assert_eq!(built, Some(serde_json::from_str::<StreamItem>(item).unwrap()), "{item}");
        }
    }

    /// A subscription id is routed alike however its text is written: as the
    /// reply wrote it, through a `Value`, or as the notification writes it.
    #[test]
    fn every_subscription_id_routes_alike_read_in_place_or_as_a_value() {
        for id in ["7", r#""s-8""#, r#""a\"b""#, r#""é""#, "12345678901234567890", "0", r#""""#] {
            let registry = Registry::new(Arc::from("kit"));
            let mut items = subscribed_as(&registry, 1, id, true);
            assert!(registry.on_text(&note(id, DONE)).is_none());
            assert!(matches!(drain_raw(&mut items)[..], [Ok(RawItem::Done)]), "{id}");
        }
    }

    /// What the spec does not excuse stays an error: an item with no `type`,
    /// or a known one missing what it must carry.
    #[test]
    fn an_item_that_is_not_one_is_still_an_error() {
        for item in [r#"{"content":{"a":1}}"#, r#"{"type":"data","metadata":{"provenance":[],"plexus_hash":"h","timestamp":1}}"#, "42"] {
            let registry = Registry::new(Arc::from("kit"));
            let mut items = subscribed(&registry, 1, "7");
            assert!(registry.on_text(&note("7", item)).is_none());
            let got = drain(&mut items);
            assert!(matches!(got[..], [Err(_)]), "{item}: {got:?}");
        }
    }

    #[test]
    fn the_subscribe_frame_is_the_one_menger_speaks() {
        let frame = subscribe_frame(1, "menger", "tickets.watch", &json!({"epic": "MACRO"}));
        let parsed: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(parsed["jsonrpc"], "2.0");
        assert_eq!(parsed["id"], 1);
        assert_eq!(parsed["method"], "menger.call");
        assert_eq!(parsed["params"]["method"], "tickets.watch");
        assert_eq!(parsed["params"]["params"], json!({"epic": "MACRO"}));
    }

    #[test]
    fn the_unsubscribe_frame_carries_the_id_positionally() {
        let frame = unsubscribe_frame(9, "menger", &json!(42));
        let parsed: Value = serde_json::from_str(&frame).unwrap();
        assert_eq!(parsed["method"], "menger.call_unsub");
        assert_eq!(parsed["params"], json!([42]));
    }

    #[test]
    fn a_numeric_and_a_string_subscription_id_do_not_collide() {
        assert_ne!(key_of(&json!(7)), key_of(&json!("7")));
    }

    /// Reply first, then drop: the caller's guard has the id, so the reader
    /// must not also cancel.
    #[test]
    fn an_acknowledged_slot_leaves_cancellation_to_the_caller() {
        let slot = SubSlot::new();
        assert!(!slot.acknowledged(&json!(1)), "nobody has dropped anything");
        assert_eq!(slot.on_dropped(), Some(json!(1)), "the caller cancels");
    }

    /// Drop first, then reply: the caller never saw an id, so the reader
    /// cancels.
    #[test]
    fn a_slot_dropped_before_the_reply_is_cancelled_by_the_reader() {
        let slot = SubSlot::new();
        assert_eq!(slot.on_dropped(), None, "no id to cancel with yet");
        assert!(slot.acknowledged(&json!(1)), "the reader cancels");
    }

    /// A stream that reached `done` has nothing to cancel either way.
    #[test]
    fn a_finished_slot_is_never_cancelled() {
        let slot = SubSlot::new();
        assert!(!slot.acknowledged(&json!(1)));
        slot.mark_finished();
        assert_eq!(slot.on_dropped(), None);

        let slot = SubSlot::new();
        slot.mark_finished();
        assert_eq!(slot.on_dropped(), None);
        assert!(!slot.acknowledged(&json!(1)));
    }
}
