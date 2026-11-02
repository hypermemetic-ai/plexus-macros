//! The transport seam: one small trait every wire implementation satisfies,
//! and the strongly-typed method reference the generated client hands it.

use crate::envelope::{RawItem, StreamItem};
use crate::error::TransportError;
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::Value;
use std::fmt;
use std::pin::Pin;

/// A boxed stream of wire items.
///
/// `Send` is required off wasm32 (so native callers can `tokio::spawn` a
/// client stream) and **not** required on wasm32, where the browser futures a
/// transport must build on (`JsFuture`, `ReadableStreamDefaultReader`) are
/// `!Send` by construction.
#[cfg(not(target_arch = "wasm32"))]
pub type ItemStream<'a> =
    Pin<Box<dyn Stream<Item = Result<StreamItem, TransportError>> + Send + 'a>>;

/// A boxed stream of wire items. See the non-wasm32 variant for the
/// `Send`-bound rationale.
#[cfg(target_arch = "wasm32")]
pub type ItemStream<'a> = Pin<Box<dyn Stream<Item = Result<StreamItem, TransportError>> + 'a>>;

/// A boxed stream of a typed client's items ([`RawItem`], PERF.115-36). The
/// same `Send` bound as [`ItemStream`].
#[cfg(not(target_arch = "wasm32"))]
pub type RawItemStream<'a> =
    Pin<Box<dyn Stream<Item = Result<RawItem, TransportError>> + Send + 'a>>;

/// A boxed stream of a typed client's items. See the non-wasm32 variant.
#[cfg(target_arch = "wasm32")]
pub type RawItemStream<'a> = Pin<Box<dyn Stream<Item = Result<RawItem, TransportError>> + 'a>>;

/// A fully qualified Plexus method — the activation namespace and the method
/// name, kept as separate fields because the transports need them
/// differently:
///
/// | Transport | Uses |
/// |---|---|
/// | WebSocket JSON-RPC | `"{path}.{method}"` as the `params.method` of a `{hub}.call` subscription |
/// | REST + SSE | `POST /rest/{namespace}/{method}` |
/// | in-process | `namespace` selects the activation, `method` is passed to `Activation::call` |
///
/// A pre-joined `&str` would force every transport to re-split it, and a
/// mis-split is silent. The generated client emits `MethodRef::new(..)` as a
/// const, so this costs nothing at the call site.
///
/// MACRO-16: a method of a `#[child]` activation is reached through its
/// parent — `tickets.inputs.answer`, not `inputs.answer`. [`MethodRef::at`]
/// gives the reference the dotted *path* the activation is mounted at
/// (`"tickets.inputs"`); [`MethodRef::qualified`] then answers
/// `"{path}.{method}"`. Without a path (every [`MethodRef::new`]) the path is
/// the namespace, exactly as before.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MethodRef {
    namespace: &'static str,
    method: &'static str,
    /// The dotted path the activation is reached at; `""` means its own
    /// namespace (a hub's top-level activation).
    path: &'static str,
}

impl MethodRef {
    pub const fn new(namespace: &'static str, method: &'static str) -> Self {
        Self { namespace, method, path: "" }
    }

    /// The same method on the activation mounted at `path` — the dotted
    /// route from the hub to it, e.g. `"tickets.inputs"` for the `inputs`
    /// child of `tickets`. A path equal to the namespace, or empty, is the
    /// top-level reference itself (so `at` never makes two equal methods
    /// compare unequal).
    pub fn at(self, path: &'static str) -> Self {
        if path.is_empty() || path == self.namespace {
            Self { path: "", ..self }
        } else {
            Self { path, ..self }
        }
    }

    /// The activation namespace, e.g. `"tickets"` (a child's own, e.g.
    /// `"inputs"`).
    pub const fn namespace(&self) -> &'static str {
        self.namespace
    }

    /// The bare method name, e.g. `"watch"`.
    pub const fn method(&self) -> &'static str {
        self.method
    }

    /// The dotted path the activation is reached at: the namespace for a
    /// top-level activation, `"tickets.inputs"` for a child.
    pub const fn path(&self) -> &'static str {
        if self.path.is_empty() {
            self.namespace
        } else {
            self.path
        }
    }

    /// The dotted form the hub's `call` method expects, e.g.
    /// `"tickets.watch"`, or `"tickets.inputs.answer"` for a child's method.
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.path(), self.method)
    }
}

impl fmt::Display for MethodRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.path(), self.method)
    }
}

/// MACRO-16: `"{parent}.{child}"` as a `&'static str`, for a generated
/// client's child accessor. A top-level parent (`""`) needs no join: the
/// path is `segment` itself. A deeper one is joined once per distinct pair
/// and kept for the life of the process — the set is the static tree of
/// `#[child]` accessors, so it is bounded by the program, not by its input.
#[doc(hidden)]
pub fn join_path(parent: &'static str, segment: &'static str) -> &'static str {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    if parent.is_empty() {
        return segment;
    }
    static PATHS: OnceLock<Mutex<HashMap<(&'static str, &'static str), &'static str>>> = OnceLock::new();
    let mut paths = PATHS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    paths
        .entry((parent, segment))
        .or_insert_with(|| Box::leak(format!("{parent}.{segment}").into_boxed_str()))
}

/// One Plexus wire implementation.
///
/// Every method call on a generated client turns into exactly one
/// `Transport::call`. The returned stream yields the response items in order
/// and ends after the terminal one (`done` or `error`); a transport that
/// loses the connection mid-stream reports it as a
/// [`TransportError`](crate::TransportError) item rather than by ending
/// silently.
///
/// Implementations must **not** interpret `params` — the generated client
/// has already built the by-name object the schema expects.
pub trait Transport {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_>;

    /// [`call`](Self::call) for a typed client (PERF.115-36): the same items,
    /// each data item's content still the wire's JSON, which
    /// [`decode_raw`](crate::decode_raw) decodes straight into the method's
    /// event. The default is `call`'s items, each content the `Value` it
    /// carries; a transport that has the wire's text (`WsTransport`) hands it
    /// over unread.
    fn call_raw(&self, method: MethodRef, params: Value) -> RawItemStream<'_> {
        Box::pin(self.call(method, params).map(|item| item.map(RawItem::from)))
    }
}

/// So a client can be handed `&T`, `Arc<T>`, `Box<T>`, … interchangeably.
impl<T: Transport + ?Sized> Transport for &T {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }

    fn call_raw(&self, method: MethodRef, params: Value) -> RawItemStream<'_> {
        (**self).call_raw(method, params)
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }

    fn call_raw(&self, method: MethodRef, params: Value) -> RawItemStream<'_> {
        (**self).call_raw(method, params)
    }
}

impl<T: Transport + ?Sized> Transport for std::rc::Rc<T> {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }

    fn call_raw(&self, method: MethodRef, params: Value) -> RawItemStream<'_> {
        (**self).call_raw(method, params)
    }
}

impl<T: Transport + ?Sized> Transport for std::sync::Arc<T> {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }

    fn call_raw(&self, method: MethodRef, params: Value) -> RawItemStream<'_> {
        (**self).call_raw(method, params)
    }
}
