//! The transport seam: one small trait every wire implementation satisfies,
//! and the strongly-typed method reference the generated client hands it.

use crate::envelope::{RawItem, StreamItem};
use crate::error::TransportError;
use futures_core::Stream;
use futures_util::StreamExt;
use serde_json::Value;
use std::fmt;
use std::pin::Pin;
use std::sync::Arc;

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

/// The dotted route a service reaches an activation at: `"tickets"`,
/// `"tickets.inputs"`, `"tickets.acme"` (a dynamic instance).
///
/// RETURNS-46: a path built at run time — a child's, a dynamic instance's —
/// is the client's own, shared by `Arc` and freed with the last client that
/// holds it: no process-wide set, nothing leaked, no lock. A path known when
/// the program is written (a namespace, a `MethodRef` constant) is a
/// `&'static str` and costs nothing; `""` means "the namespace". So it is
/// `Cow<'static, str>` with an `Arc` arm: a constant stays `const`, and a
/// clone of a run-time path is a reference count, not a copy.
#[derive(Clone)]
pub struct RoutePath(PathRepr);

#[derive(Clone)]
enum PathRepr {
    Static(&'static str),
    Shared(Arc<str>),
}

impl RoutePath {
    /// The empty path: the activation's own namespace.
    pub const EMPTY: RoutePath = RoutePath(PathRepr::Static(""));

    /// A path known when the program is written.
    pub const fn from_static(path: &'static str) -> Self {
        RoutePath(PathRepr::Static(path))
    }

    /// The path as text.
    pub fn as_str(&self) -> &str {
        match &self.0 {
            PathRepr::Static(s) => s,
            PathRepr::Shared(s) => s,
        }
    }

    /// `true` for the empty path.
    pub fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }

    /// `"{self}.{segment}"`, or `segment` when `self` is empty — one
    /// allocation, the new path's own.
    pub fn join(&self, segment: &str) -> RoutePath {
        if self.is_empty() {
            return RoutePath::from(segment.to_owned());
        }
        let mut s = String::with_capacity(self.as_str().len() + 1 + segment.len());
        s.push_str(self.as_str());
        s.push('.');
        s.push_str(segment);
        RoutePath::from(s)
    }
}

impl From<&'static str> for RoutePath {
    fn from(s: &'static str) -> Self {
        RoutePath::from_static(s)
    }
}

impl From<String> for RoutePath {
    fn from(s: String) -> Self {
        RoutePath(PathRepr::Shared(Arc::from(s)))
    }
}

impl From<Arc<str>> for RoutePath {
    fn from(s: Arc<str>) -> Self {
        RoutePath(PathRepr::Shared(s))
    }
}

impl PartialEq for RoutePath {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for RoutePath {}

impl std::hash::Hash for RoutePath {
    fn hash<H: std::hash::Hasher>(&self, h: &mut H) {
        self.as_str().hash(h)
    }
}

impl fmt::Debug for RoutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for RoutePath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A fully qualified Plexus method — the activation namespace and the method
/// name, kept as separate fields because the transports need them
/// differently:
///
/// | Transport | Uses |
/// |---|---|
/// | WebSocket JSON-RPC | `"{path}.{method}"` as the `params.method` of a `{service}.call` subscription |
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
///
/// RETURNS-46: the path is a [`RoutePath`] — a dynamic instance's path is a
/// run-time value — so a `MethodRef` is `Clone`, not `Copy`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct MethodRef {
    namespace: &'static str,
    method: &'static str,
    /// The dotted path the activation is reached at; empty means its own
    /// namespace (a service's top-level activation).
    path: RoutePath,
}

impl MethodRef {
    pub const fn new(namespace: &'static str, method: &'static str) -> Self {
        Self { namespace, method, path: RoutePath::EMPTY }
    }

    /// The same method on the activation mounted at `path` — the dotted
    /// route from the service to it, e.g. `"tickets.inputs"` for the
    /// `inputs` child of `tickets`. A path equal to the namespace, or empty,
    /// is the top-level reference itself (so `at` never makes two equal
    /// methods compare unequal).
    pub fn at(self, path: impl Into<RoutePath>) -> Self {
        let path = path.into();
        if path.is_empty() || path.as_str() == self.namespace {
            Self { path: RoutePath::EMPTY, ..self }
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
    pub fn path(&self) -> &str {
        if self.path.is_empty() {
            self.namespace
        } else {
            self.path.as_str()
        }
    }

    /// The dotted form the service's `call` method expects, e.g.
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
