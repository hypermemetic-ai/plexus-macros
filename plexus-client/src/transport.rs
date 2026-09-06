//! The transport seam: one small trait every wire implementation satisfies,
//! and the strongly-typed method reference the generated client hands it.

use crate::envelope::StreamItem;
use crate::error::TransportError;
use futures_core::Stream;
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

/// A fully qualified Plexus method — the activation namespace and the method
/// name, kept as separate fields because the transports need them
/// differently:
///
/// | Transport | Uses |
/// |---|---|
/// | WebSocket JSON-RPC | `"{namespace}.{method}"` as the `params.method` of a `{hub}.call` subscription |
/// | REST + SSE | `POST /rest/{namespace}/{method}` |
/// | in-process | `namespace` selects the activation, `method` is passed to `Activation::call` |
///
/// A pre-joined `&str` would force every transport to re-split it, and a
/// mis-split is silent. The generated client emits `MethodRef::new(..)` as a
/// const, so this costs nothing at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct MethodRef {
    namespace: &'static str,
    method: &'static str,
}

impl MethodRef {
    pub const fn new(namespace: &'static str, method: &'static str) -> Self {
        Self { namespace, method }
    }

    /// The activation namespace, e.g. `"tickets"`.
    pub const fn namespace(&self) -> &'static str {
        self.namespace
    }

    /// The bare method name, e.g. `"watch"`.
    pub const fn method(&self) -> &'static str {
        self.method
    }

    /// The dotted form the hub's `call` method expects, e.g.
    /// `"tickets.watch"`.
    pub fn qualified(&self) -> String {
        format!("{}.{}", self.namespace, self.method)
    }
}

impl fmt::Display for MethodRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.namespace, self.method)
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
}

/// So a client can be handed `&T`, `Arc<T>`, `Box<T>`, … interchangeably.
impl<T: Transport + ?Sized> Transport for &T {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }
}

impl<T: Transport + ?Sized> Transport for Box<T> {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }
}

impl<T: Transport + ?Sized> Transport for std::rc::Rc<T> {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }
}

impl<T: Transport + ?Sized> Transport for std::sync::Arc<T> {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        (**self).call(method, params)
    }
}
