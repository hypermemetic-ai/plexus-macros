//! LOCAL-3 (spike, never merged): the calls a peer serves on a connection
//! it opened — the callee's half of a symmetric connection, with the frames
//! a service uses for the calls it serves:
//!
//! ```text
//! ← {"jsonrpc":"2.0","id":1,"method":"box2.call","params":{"method":"ci.run","params":{…},"hop":{…}}}
//! → {"jsonrpc":"2.0","id":1,"result":1}
//! → {"jsonrpc":"2.0","method":"box2.call","params":{"subscription":1,"result":{"type":"credit","n":64}}}
//! ← {"jsonrpc":"2.0","method":"box2.input","params":{"subscription":1,"seq":0,"items":[…]}}
//! → {"jsonrpc":"2.0","method":"box2.call","params":{"subscription":1,"result":{"type":"ack","seq":0}}}
//! ← {"jsonrpc":"2.0","method":"box2.end","params":{"subscription":1,"seq":1}}
//! → … {"type":"data",…} … {"type":"done"}
//! ← {"jsonrpc":"2.0","id":2,"method":"box2.call_unsub","params":[1]}
//! → {"jsonrpc":"2.0","id":2,"result":true}
//! ```
//!
//! Subscription ids here are minted by this side, from 1. They live in a
//! table of their own — the calls this side serves — apart from the
//! registry's table of calls this side opened (ids the other side minted),
//! and a frame's shape says which table it names (`input`, `end`,
//! `call_unsub` → served; an item, `params.result` → opened). So the two
//! sides may mint the same numbers at once and nothing collides.

use crate::offer::{Hop, Offered, ServedCall};
use futures_util::stream::{BoxStream, Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::{BTreeMap, HashMap};
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use tokio::sync::mpsc;

use super::Writer;

/// Counts a test reads.
#[derive(Debug, Default)]
pub struct ServedStats {
    /// Calls open now.
    pub open: AtomicU64,
    /// Calls ever opened.
    pub opened: AtomicU64,
    /// Calls refused `not_offered` (no handler ran).
    pub not_offered: AtomicU64,
    /// Input frames for a subscription this side does not serve.
    pub unknown: AtomicU64,
    /// Input frames at or below the delivered seq (a resend).
    pub duplicates: AtomicU64,
    /// Cancels received (`call_unsub`).
    pub cancelled: AtomicU64,
}

struct InputState {
    tx: Option<mpsc::UnboundedSender<Value>>,
    next_seq: u64,
    pending: BTreeMap<u64, Vec<Value>>,
    end_at: Option<u64>,
    received: u64,
    consumed: u64,
    granted: u64,
    aborted: bool,
}

struct Call {
    input: Mutex<InputState>,
    ctl: mpsc::UnboundedSender<Ctl>,
    abort: Mutex<Option<tokio::task::AbortHandle>>,
}

enum Ctl {
    Item(Value),
    /// Write it, then end the call.
    End(Value),
}

pub(crate) struct Served {
    ns: String,
    offered: Arc<dyn Offered>,
    write: Writer,
    window: u64,
    calls: Mutex<HashMap<u64, Arc<Call>>>,
    next_sub: AtomicU64,
    closed: std::sync::atomic::AtomicBool,
    pub(crate) stats: ServedStats,
}

impl std::fmt::Debug for Served {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Served").field("ns", &self.ns).field("open", &self.stats.open).finish()
    }
}

fn item_frame(ns: &str, sub: u64, item: &Value) -> String {
    json!({"jsonrpc": "2.0", "method": format!("{ns}.call"), "params": {"subscription": sub, "result": item}}).to_string()
}

fn is_terminal(item: &Value) -> bool {
    matches!(item.get("type").and_then(Value::as_str), Some("done") | Some("error"))
}

impl Served {
    pub(crate) fn new(offered: Arc<dyn Offered>, write: Writer, window: u64) -> Arc<Self> {
        Arc::new(Self {
            ns: offered.namespace().to_string(),
            offered,
            write,
            window,
            calls: Mutex::new(HashMap::new()),
            next_sub: AtomicU64::new(1),
            closed: Default::default(),
            stats: ServedStats::default(),
        })
    }

    pub(crate) fn namespace(&self) -> &str {
        &self.ns
    }

    pub(crate) fn open_calls(&self) -> usize {
        self.calls.lock().unwrap().len()
    }

    /// A request (it has an `id`) naming this side's namespace. `true` when
    /// it was this side's to answer.
    pub(crate) fn on_request(self: &Arc<Self>, method: &str, frame: &Value) -> bool {
        let Some(verb) = method.strip_prefix(&self.ns).and_then(|r| r.strip_prefix('.')) else {
            return false;
        };
        let id = frame.get("id").cloned().unwrap_or(Value::Null);
        match verb {
            "call" => self.open(id, frame.get("params").cloned().unwrap_or(Value::Null)),
            "call_unsub" => {
                let sub = frame.pointer("/params/0").and_then(Value::as_u64);
                let call = sub.and_then(|s| self.calls.lock().unwrap().remove(&s));
                let found = call.is_some();
                if let Some(call) = call {
                    self.stats.cancelled.fetch_add(1, Ordering::Relaxed);
                    self.stats.open.fetch_sub(1, Ordering::Relaxed);
                    call.input.lock().unwrap().aborted = true;
                    if let Some(a) = call.abort.lock().unwrap().take() {
                        a.abort();
                    }
                }
                let _ = (self.write)(json!({"jsonrpc": "2.0", "id": id, "result": found}).to_string());
            }
            _ => {
                let _ = (self.write)(
                    json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": format!("Method not found: {method}")}})
                        .to_string(),
                );
            }
        }
        true
    }

    /// `{ns}.input {subscription, seq, items}` / `{ns}.end {subscription, seq}`.
    pub(crate) fn on_input(&self, method: &str, frame: &Value) -> bool {
        let Some(verb) = method.strip_prefix(&self.ns).and_then(|r| r.strip_prefix('.')) else {
            return false;
        };
        if verb != "input" && verb != "end" {
            return false;
        }
        let params = frame.get("params").cloned().unwrap_or(Value::Null);
        let Some(sub) = params.get("subscription").and_then(Value::as_u64) else { return true };
        let Some(call) = self.calls.lock().unwrap().get(&sub).cloned() else {
            self.stats.unknown.fetch_add(1, Ordering::Relaxed);
            return true;
        };
        let seq = params.get("seq").and_then(Value::as_u64).unwrap_or(0);
        if verb == "end" {
            let mut st = call.input.lock().unwrap();
            if st.next_seq >= seq {
                st.tx = None;
            } else {
                st.end_at = Some(seq);
            }
            return true;
        }
        let items = params.get("items").and_then(Value::as_array).cloned().unwrap_or_default();
        let mut st = call.input.lock().unwrap();
        if st.aborted || st.tx.is_none() {
            return true;
        }
        if seq < st.next_seq || st.pending.contains_key(&seq) {
            self.stats.duplicates.fetch_add(1, Ordering::Relaxed);
            return true;
        }
        st.received += items.len() as u64;
        if st.received > st.granted {
            st.aborted = true;
            st.tx = None;
            let _ = call.ctl.send(Ctl::End(json!({
                "type": "error", "code": "past_credit", "error_kind": "past_credit",
                "message": format!("seq {seq}: {} items past a grant of {}", st.received - st.granted, st.granted),
                "seq": seq, "credit": st.granted,
            })));
            return true;
        }
        st.pending.insert(seq, items);
        loop {
            let next = st.next_seq;
            let Some(batch) = st.pending.remove(&next) else { break };
            if let Some(tx) = &st.tx {
                for it in batch {
                    let _ = tx.send(it);
                }
            }
            st.next_seq += 1;
        }
        if st.end_at.is_some_and(|e| st.next_seq >= e) {
            st.tx = None;
        }
        true
    }

    fn open(self: &Arc<Self>, id: Value, params: Value) {
        let method = params.get("method").and_then(Value::as_str).unwrap_or_default().to_string();
        let activation = method.split('.').next().unwrap_or_default();
        let sub = self.next_sub.fetch_add(1, Ordering::Relaxed);
        let _ = (self.write)(json!({"jsonrpc": "2.0", "id": id, "result": sub}).to_string());

        if !self.offered.activations().iter().any(|a| a == activation) {
            // The callee's refusal, typed, before anything runs — as a gate's
            // refusal goes on the forward direction: the subscription, then
            // one terminal error item.
            self.stats.not_offered.fetch_add(1, Ordering::Relaxed);
            let item = json!({
                "type": "error", "code": "not_offered", "error_kind": "forbidden",
                "message": format!("forbidden: `{method}` is not offered on this connection (not_offered)"),
                "recoverable": false,
            });
            let _ = (self.write)(item_frame(&self.ns, sub, &item));
            return;
        }

        let hop: Option<Hop> = params.get("hop").and_then(|h| serde_json::from_value(h.clone()).ok());
        let has_input = self.offered.has_input(&method);
        let (ctl_tx, mut ctl_rx) = mpsc::unbounded_channel::<Ctl>();
        let (in_tx, in_rx) = mpsc::unbounded_channel::<Value>();
        let call = Arc::new(Call {
            input: Mutex::new(InputState {
                tx: has_input.then_some(in_tx),
                next_seq: 0,
                pending: BTreeMap::new(),
                end_at: None,
                received: 0,
                consumed: 0,
                granted: if has_input { self.window } else { 0 },
                aborted: false,
            }),
            ctl: ctl_tx,
            abort: Mutex::new(None),
        });
        self.calls.lock().unwrap().insert(sub, call.clone());
        self.stats.open.fetch_add(1, Ordering::Relaxed);
        self.stats.opened.fetch_add(1, Ordering::Relaxed);

        let input: Option<BoxStream<'static, Value>> = has_input
            .then(|| InputItems { rx: in_rx, call: call.clone(), window: self.window }.boxed());
        let mut out = self.offered.serve(ServedCall { method, params: params.get("params").cloned().unwrap_or(Value::Null), hop, input });
        let me = self.clone();
        let window = self.window;
        let task = tokio::spawn(async move {
            let ns = me.ns.clone();
            let write = |v: &Value| (me.write)(item_frame(&ns, sub, v));
            if has_input && write(&json!({"type": "credit", "n": window})).is_err() {
                return;
            }
            loop {
                tokio::select! {
                    biased;
                    c = ctl_rx.recv() => match c {
                        Some(Ctl::Item(v)) => { if write(&v).is_err() { break } }
                        Some(Ctl::End(v)) => { let _ = write(&v); break }
                        None => break,
                    },
                    o = out.next() => match o {
                        Some(item) => {
                            let end = is_terminal(&item);
                            if end {
                                while let Ok(Ctl::Item(v)) = ctl_rx.try_recv() { let _ = write(&v); }
                                let open = {
                                    let mut st = call.input.lock().unwrap();
                                    let open = st.tx.take().is_some();
                                    if open { Some(st.next_seq.checked_sub(1)) } else { None }
                                };
                                if let Some(ack) = open {
                                    let _ = write(&json!({"type": "input_closed", "reason": "method_ended", "ack": ack}));
                                }
                            }
                            if write(&item).is_err() || end { break }
                        }
                        None => {
                            let _ = write(&json!({"type": "done"}));
                            break;
                        }
                    },
                }
            }
            drop(out);
            if me.calls.lock().unwrap().remove(&sub).is_some() {
                me.stats.open.fetch_sub(1, Ordering::Relaxed);
            }
        });
        if let Some(c) = self.calls.lock().unwrap().get(&sub) {
            *c.abort.lock().unwrap() = Some(task.abort_handle());
        }
    }

    /// This side ends every call it serves: each gets one terminal error
    /// (`offer_withdrawn`), and its handler is dropped.
    pub(crate) fn withdraw(&self) {
        let calls: Vec<_> = self.calls.lock().unwrap().values().cloned().collect();
        for call in calls {
            let _ = call.ctl.send(Ctl::End(json!({
                "type": "error", "code": "offer_withdrawn", "error_kind": "offer_withdrawn",
                "message": "the peer withdrew its offer", "recoverable": false,
            })));
        }
    }

    /// The socket is gone: every served call's handler is dropped, nothing
    /// is kept.
    pub(crate) fn on_close(&self) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        let calls: Vec<_> = self.calls.lock().unwrap().drain().map(|(_, c)| c).collect();
        for call in calls {
            self.stats.open.fetch_sub(1, Ordering::Relaxed);
            call.input.lock().unwrap().aborted = true;
            if let Some(a) = call.abort.lock().unwrap().take() {
                a.abort();
            }
        }
    }
}

/// The handler's input: items in `seq` order; each taken grants credit back
/// in half windows, with the ack — BIDIR-3's rule, the other way round.
struct InputItems {
    rx: mpsc::UnboundedReceiver<Value>,
    call: Arc<Call>,
    window: u64,
}

impl Stream for InputItems {
    type Item = Value;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Value>> {
        let p = self.rx.poll_recv(cx);
        if let Poll::Ready(Some(_)) = &p {
            let mut st = self.call.input.lock().unwrap();
            st.consumed += 1;
            let ceiling = st.consumed + self.window;
            let n = ceiling - st.granted;
            if n >= (self.window / 2).max(1) && !st.aborted {
                st.granted = ceiling;
                let ack = st.next_seq.checked_sub(1);
                let _ = self.call.ctl.send(Ctl::Item(json!({"type": "ack", "seq": ack})));
                let _ = self.call.ctl.send(Ctl::Item(json!({"type": "credit", "n": n})));
            }
        }
        p
    }
}
