//! The backend-independent half of the WebSocket transport: request ids,
//! the pending-subscribe table, the subscription routing table, and the
//! two frame shapes.
//!
//! Everything here is pure state plus `serde_json` — no socket, no runtime.
//! That is what lets the native (`tokio-tungstenite`) and browser
//! (`web_sys::WebSocket`) backends share one protocol implementation
//! instead of two that drift.

use crate::envelope::StreamItem;
use crate::error::TransportError;
use futures_channel::{mpsc, oneshot};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

/// The receiving half of one subscription's item channel.
pub(crate) type Items = mpsc::UnboundedReceiver<Inbound>;
type ItemsTx = mpsc::UnboundedSender<Inbound>;

/// What arrives for a subscription.
#[derive(Debug, PartialEq)]
pub(crate) enum Inbound {
    /// A stream item, or the transport failing.
    Item(Result<StreamItem, TransportError>),
    /// BIDIR-6: a frame of the call's input half. This transport's to spend,
    /// so it is no [`StreamItem`]: a caller never sees one.
    Input(InputSignal),
}

/// The input half's output items.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum InputSignal {
    /// `credit {n}`: `n` more items.
    Credit(u64),
    /// `input_closed`: the callee reads no more.
    Closed,
    /// `ack`: nothing for a sender that never resends.
    Ack,
}

impl InputSignal {
    fn of(result: &Value) -> Option<Self> {
        match result.get("type")?.as_str()? {
            "credit" => Some(Self::Credit(result.get("n")?.as_u64()?)),
            "input_closed" => Some(Self::Closed),
            "ack" => Some(Self::Ack),
            _ => None,
        }
    }
}

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
    /// The subscription id, once acknowledged.
    pub(crate) fn subscription(&self) -> Option<Value> {
        self.lock().subscription.clone()
    }

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
}

#[derive(Debug, Default)]
struct State {
    /// JSON-RPC request id → the call waiting for that subscribe's reply.
    pending: HashMap<u64, Pending>,
    /// subscription key → that subscription's sink and cancellation state.
    subs: HashMap<String, Sub>,
    /// BIDIR-6: items for a subscription not learned yet, kept while a
    /// subscribe is outstanding: a server's first `credit` may reach a client
    /// that has not read the id it follows. Bounded: past [`EARLY_MOST`] for a
    /// subscription they are lost, as an unknown subscription's always were.
    early: HashMap<String, Vec<Value>>,
    /// Set once, when the socket goes away.
    closed: Option<TransportError>,
}

/// The items kept for one subscription not learned yet.
const EARLY_MOST: usize = 64;

/// One connection's protocol state.
#[derive(Debug)]
pub(crate) struct Registry {
    hub_namespace: Arc<str>,
    next_id: AtomicU64,
    state: Mutex<State>,
}

/// The routing key for a subscription id.
///
/// Rendered as JSON text rather than "the string if it is one", so a server
/// that answers `7` and a server that answers `"7"` cannot collide.
fn key_of(id: &Value) -> String {
    id.to_string()
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
    ) -> Result<oneshot::Receiver<Result<Items, TransportError>>, TransportError> {
        let mut state = self.lock();
        if let Some(err) = &state.closed {
            return Err(err.clone());
        }
        let (waiter, reply) = oneshot::channel();
        state.pending.insert(request_id, Pending { slot, waiter });
        Ok(reply)
    }

    /// Give up on a subscribe whose frame could not be written.
    pub(crate) fn forget_pending(&self, request_id: u64) {
        self.lock().pending.remove(&request_id);
    }

    /// Drop a subscription's routing entry (the stream was dropped, or the
    /// server sent its terminal item).
    pub(crate) fn forget_sub(&self, subscription: &Value) {
        self.lock().subs.remove(&key_of(subscription));
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
    pub(crate) fn on_text(&self, text: &str) -> Option<String> {
        let frame = serde_json::from_str::<Value>(text).ok()?;

        // A subscribe reply.
        if let Some(request_id) = frame.get("id").and_then(Value::as_u64) {
            if frame.get("result").is_some() || frame.get("error").is_some() {
                return self.on_response(request_id, &frame);
            }
        }

        // A stream item.
        if let Some(subscription) = frame.pointer("/params/subscription") {
            self.on_notification(&subscription.clone(), &frame);
        }
        None
    }

    fn on_response(&self, request_id: u64, frame: &Value) -> Option<String> {
        let mut state = self.lock();
        // Not ours: the reply to an unsubscribe, or a duplicate.
        let pending = state.pending.remove(&request_id)?;

        if let Some(error) = frame.get("error") {
            drop(state);
            let _ = pending.waiter.send(Err(TransportError::protocol(format!(
                "subscribe rejected: {error}"
            ))));
            return None;
        }

        let subscription = frame.get("result").cloned().unwrap_or(Value::Null);

        // The caller dropped its stream before the acknowledgement arrived,
        // so it never learned this id and could not cancel it. Cancel it
        // here instead of registering a channel nobody will read.
        if pending.slot.acknowledged(&subscription) {
            drop(state);
            return Some(self.unsubscribe(&subscription));
        }

        let key = key_of(&subscription);
        let (sink, items) = mpsc::unbounded();
        state.subs.insert(
            key.clone(),
            Sub {
                sink,
                slot: pending.slot.clone(),
            },
        );
        let early = state.early.remove(&key).unwrap_or_default();
        if state.pending.is_empty() {
            state.early.clear();
        }
        drop(state);

        for frame in &early {
            self.on_notification(&subscription, frame);
        }
        if pending.waiter.send(Ok(items)).is_err() {
            // Dropped in the instant between `acknowledged` and here; the
            // stream's own guard has the id and is cancelling it. Just stop
            // routing to a receiver that is gone.
            self.lock().subs.remove(&key);
        }
        None
    }

    fn on_notification(&self, subscription: &Value, frame: &Value) {
        let key = key_of(subscription);
        let mut state = self.lock();
        let Some((sink, slot)) = state
            .subs
            .get(&key)
            .map(|sub| (sub.sink.clone(), sub.slot.clone()))
        else {
            if !state.pending.is_empty() {
                let held = state.early.entry(key).or_default();
                if held.len() < EARLY_MOST {
                    held.push(frame.clone());
                }
            }
            return;
        };
        drop(state);

        let payload = frame.pointer("/params/result").cloned().unwrap_or(Value::Null);
        if let Some(signal) = InputSignal::of(&payload) {
            let _ = sink.unbounded_send(Inbound::Input(signal));
            return;
        }
        match serde_json::from_value::<StreamItem>(payload) {
            Ok(item) => {
                let terminal = item.is_terminal();
                let _ = sink.unbounded_send(Inbound::Item(Ok(item)));
                if terminal {
                    // The server ended it; there is nothing left to cancel,
                    // whether or not the caller ever reads this item.
                    slot.mark_finished();
                    self.forget_sub(subscription);
                }
            }
            Err(e) => {
                let _ = sink.unbounded_send(Inbound::Item(Err(TransportError::protocol(format!(
                    "notification payload is not a Plexus stream item: {e}"
                )))));
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
            (
                std::mem::take(&mut state.pending),
                std::mem::take(&mut state.subs),
            )
        };

        for (_, waiting) in pending {
            let _ = waiting.waiter.send(Err(error.clone()));
        }
        for (_, sub) in subs {
            let _ = sub.sink.unbounded_send(Inbound::Item(Err(error.clone())));
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

/// BIDIR-6: `<hub>.input`, a notification: `items` is the items' JSON,
/// comma-separated, already written.
pub(crate) fn input_frame(hub_namespace: &str, subscription: &Value, seq: u64, items: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","method":{},"params":{{"subscription":{subscription},"seq":{seq},"items":[{items}]}}}}"#,
        Value::from(format!("{hub_namespace}.input"))
    )
}

/// BIDIR-6: `<hub>.end`: `seq` is the number of input frames sent.
pub(crate) fn end_frame(hub_namespace: &str, subscription: &Value, seq: u64) -> String {
    json!({
        "jsonrpc": "2.0",
        "method": format!("{hub_namespace}.end"),
        "params": { "subscription": subscription, "seq": seq },
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

    #[test]
    fn the_input_frames_are_the_ones_the_wire_says() {
        let frame: Value = serde_json::from_str(&input_frame("menger", &json!(21), 0, r#"{"t":"The"},{"t":" cat"}"#)).unwrap();
        assert_eq!(frame, json!({"jsonrpc":"2.0","method":"menger.input","params":{"subscription":21,"seq":0,"items":[{"t":"The"},{"t":" cat"}]}}));
        let frame: Value = serde_json::from_str(&end_frame("menger", &json!(21), 2)).unwrap();
        assert_eq!(frame, json!({"jsonrpc":"2.0","method":"menger.end","params":{"subscription":21,"seq":2}}));
    }

    /// BIDIR-6: an item that reaches a client before the id it follows is
    /// kept for the call that learns it.
    #[test]
    fn an_item_before_its_subscribe_reply_is_held_for_it() {
        let registry = Registry::new(Arc::from("menger"));
        let slot = SubSlot::new();
        let id = registry.next_request_id();
        let mut reply = registry.register_pending(id, slot).unwrap();
        let credit = json!({"jsonrpc":"2.0","method":"result","params":{"subscription":21,"result":{"type":"credit","n":64}}}).to_string();
        assert!(registry.on_text(&credit).is_none());
        let answer = json!({"jsonrpc":"2.0","id":id,"result":21}).to_string();
        assert!(registry.on_text(&answer).is_none());
        let mut items = reply.try_recv().unwrap().unwrap().unwrap();
        assert_eq!(items.try_recv().unwrap(), Inbound::Input(InputSignal::Credit(64)));
    }

    /// With nothing outstanding an unknown subscription's item is dropped,
    /// as ever: nothing grows.
    #[test]
    fn an_item_of_no_subscription_is_not_kept() {
        let registry = Registry::new(Arc::from("menger"));
        let stray = json!({"jsonrpc":"2.0","method":"result","params":{"subscription":9,"result":{"type":"done"}}}).to_string();
        assert!(registry.on_text(&stray).is_none());
        assert!(registry.lock().early.is_empty());
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
