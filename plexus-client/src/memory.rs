//! `MemoryTransport` — a scripted [`Transport`] with no server behind it.
//!
//! This is the transport a *client* test uses. It is wasm-safe (it is the
//! only transport that can be, since it has no I/O), so it is what proves a
//! generated client compiles and runs for `wasm32-unknown-unknown` without
//! also standing up a browser.
//!
//! It is not the in-process transport: that one drives a real `Activation`
//! and therefore lives on the `plexus-core` side. This one replays a script.

use crate::envelope::StreamItem;
use crate::error::TransportError;
use crate::transport::{ItemStream, MethodRef, Transport};
use futures_util::stream;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

type Script = Vec<Result<StreamItem, TransportError>>;

/// A [`Transport`] that answers from a script and records what it was asked.
///
/// ```
/// use futures::TryStreamExt;
/// use plexus_client::{decode, MemoryTransport, MethodRef, Transport};
///
/// #[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
/// struct Ev { n: u8 }
///
/// const COUNT: MethodRef = MethodRef::new("demo", "count");
///
/// let t = MemoryTransport::new().script_events(COUNT, &[Ev { n: 1 }, Ev { n: 2 }]);
/// let events = futures::executor::block_on(async {
///     decode::<Ev>(t.call(COUNT, serde_json::json!({}))).try_collect::<Vec<_>>().await
/// })
/// .unwrap();
///
/// assert_eq!(events, vec![Ev { n: 1 }, Ev { n: 2 }]);
/// assert_eq!(t.params_of(COUNT), Some(serde_json::json!({})));
/// ```
#[derive(Debug, Default)]
pub struct MemoryTransport {
    scripts: HashMap<String, Script>,
    /// Every `(method, params)` this transport was called with, in order.
    calls: Mutex<Vec<(String, Value)>>,
}

impl MemoryTransport {
    pub fn new() -> Self {
        Self::default()
    }

    /// Script the raw wire items a method answers with. Use this to exercise
    /// the shapes only a real transport produces — `progress`, an in-band
    /// `error`, a `data` item with the wrong payload.
    pub fn script(mut self, method: MethodRef, items: Vec<StreamItem>) -> Self {
        self.scripts
            .insert(method.qualified(), items.into_iter().map(Ok).collect());
        self
    }

    /// Script a transport failure — the connection dying instead of the
    /// server answering.
    pub fn script_failure(mut self, method: MethodRef, error: TransportError) -> Self {
        self.scripts.insert(method.qualified(), vec![Err(error)]);
        self
    }

    /// Script the happy path: each event becomes a `data` item stamped with
    /// the method's `content_type`, followed by `done`.
    ///
    /// Serialization uses the event type's own `Serialize`, so a script
    /// written against the server's event enum cannot drift from what the
    /// server would actually put on the wire.
    pub fn script_events<E: Serialize>(self, method: MethodRef, events: &[E]) -> Self {
        let content_type = method.qualified();
        let mut items: Vec<StreamItem> = events
            .iter()
            .map(|e| StreamItem::Data {
                metadata: None,
                content_type: Some(content_type.clone()),
                content: serde_json::to_value(e).expect("event serializes"),
            })
            .collect();
        items.push(StreamItem::done());
        self.script(method, items)
    }

    /// Every call this transport received, in order, as
    /// `("namespace.method", params)`.
    pub fn calls(&self) -> Vec<(String, Value)> {
        self.calls.lock().expect("calls lock").clone()
    }

    /// The params of the single call to `method`, or `None` if it was not
    /// called exactly once.
    pub fn params_of(&self, method: MethodRef) -> Option<Value> {
        let q = method.qualified();
        let calls = self.calls.lock().expect("calls lock");
        let mut hits = calls.iter().filter(|(m, _)| *m == q);
        let first = hits.next()?;
        match hits.next() {
            None => Some(first.1.clone()),
            Some(_) => None,
        }
    }
}

impl Transport for MemoryTransport {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        let qualified = method.qualified();
        self.calls
            .lock()
            .expect("calls lock")
            .push((qualified.clone(), params));

        let script = self.scripts.get(&qualified).cloned().unwrap_or_else(|| {
            vec![Err(TransportError::protocol(format!(
                "MemoryTransport has no script for `{qualified}`"
            )))]
        });
        Box::pin(stream::iter(script))
    }
}
