//! NEST-14 · a client for a node whose interface is not known when the
//! program is written: an untyped peer slot, a `#[child(opaque)]`, a tree
//! mounted at run time.
//!
//! [`Untyped`] is a transport and a dotted path. It calls a method by name
//! ([`Untyped::call`], [`Untyped::call_one`]), reaches a node below by name
//! ([`Untyped::child`]), asks the node's schema ([`Untyped::schema`]), and —
//! once the caller knows the interface — becomes that interface's generated
//! client, checked against the node's schema first ([`Untyped::typed`]):
//!
//! ```ignore
//! let shop = dyn_client.peer("shop");                 // Untyped<&T>
//! let shop: ShopClient<_> = shop.typed().await?;      // ShapeMismatch if it is not a shop
//! let items = shop.items().list().await?;
//! ```
//!
//! Its items are `serde_json::Value`s; nothing lifts them into a parent's
//! aggregate (a typed parent has no variant for a node it cannot type).
//! Works over every [`Transport`]: a socket, a service in process, a
//! host's transport handed to a child.

use std::fmt;

use serde_json::Value;

use crate::error::ClientError;
use crate::method_stream::{call_one, MethodStream};
use crate::route::Route;
use crate::transport::{MethodRef, RoutePath, Transport};
use crate::Content;

/// A generated client made at a route: what [`Untyped::typed`] turns an
/// untyped client into. The activation macro implements it for every client
/// it emits (`XClient<T>`, its events its own).
pub trait FromRoute<T>: Sized {
    /// The client of the node at `route`, over `transport`.
    fn from_route(transport: T, route: Route) -> Self;
}

/// What a generated client was built against, checked against a node's
/// schema: the methods it calls, each by its shape's hash. The activation
/// macro implements it beside the server half (where the method schemas
/// are), so a client-only build (wasm32) has [`Untyped::typed_unchecked`]
/// alone.
pub trait ShapeCheck {
    /// `Ok` when `schema` — the node at `path`'s answer to `schema` — serves
    /// every method this client calls under the same shape; else
    /// [`ClientError::ShapeMismatch`] naming those it does not.
    fn shape_check(path: &str, schema: &Value) -> Result<(), ClientError>;
}

/// An untyped client: a transport and the dotted path of a node.
pub struct Untyped<T> {
    transport: T,
    route: Route,
}

impl<T> Untyped<T> {
    /// The node at the dotted `path` (`"host.dendrite.digest"`); empty is
    /// the service itself.
    pub fn new(transport: T, path: impl Into<RoutePath>) -> Self {
        Self { transport, route: Route::at(path, "") }
    }

    /// The node at `route` — what a generated accessor builds; a refused
    /// route makes a client every call of which fails at once.
    pub fn from_route(transport: T, route: Route) -> Self {
        Self { transport, route }
    }

    /// The node's dotted path.
    pub fn path(&self) -> &str {
        self.route.path("")
    }

    /// The node's route.
    pub fn route(&self) -> &Route {
        &self.route
    }

    /// Borrow the transport.
    pub fn transport(&self) -> &T {
        &self.transport
    }

    /// Unwrap the transport.
    pub fn into_transport(self) -> T {
        self.transport
    }

    /// The node `name` below this one (`<path>.<name>`): a static child, an
    /// instance of a dynamic one, a peer slot's name. A name that is not a
    /// route segment makes a client every call of which fails at once with
    /// [`ClientError::InvalidName`].
    pub fn child(&self, name: &str) -> Untyped<&T> {
        Untyped { transport: &self.transport, route: self.route.segment(name) }
    }

    /// [`Self::child`], keeping the transport.
    pub fn into_child(self, name: &str) -> Untyped<T> {
        let route = self.route.segment(name);
        Untyped { transport: self.transport, route }
    }

    /// The generated client `C` of this node, unchecked — for a caller that
    /// knows the interface by other means, or a build with no
    /// [`ShapeCheck`] (a client-only one).
    pub fn typed_unchecked<C: FromRoute<T>>(self) -> C {
        C::from_route(self.transport, self.route)
    }

    fn method(&self, method: &str) -> MethodRef {
        MethodRef::named(RoutePath::from(self.path().to_owned()), RoutePath::from(method.to_owned()))
    }
}

impl<T: Transport> Untyped<T> {
    /// Call `method` on this node with `params` (an object, by name): its
    /// items as JSON, `Err` for the call failing, progress dropped
    /// (`.with_progress()` keeps it).
    pub fn call(&self, method: &str, params: Value) -> MethodStream<'_, Value> {
        let params = match self.route.refusal() {
            None => Ok(params),
            Some(e) => Err(e),
        };
        MethodStream::call(&self.transport, self.method(method), params, Content::decode::<Value>, |v| v)
    }

    /// Call a one-item `method`: its item, or why there is none.
    pub async fn call_one(&self, method: &str, params: Value) -> Result<Value, ClientError> {
        let params = match self.route.refusal() {
            None => Ok(params),
            Some(e) => Err(e),
        };
        call_one(&self.transport, self.method(method), params, Content::decode::<Value>).await
    }

    /// The node's schema, as it answers `<path>.schema`.
    pub async fn schema(&self) -> Result<Value, ClientError> {
        self.call_one("schema", Value::Object(Default::default())).await
    }

    /// This node as the generated client `C`, once its schema says it
    /// serves `C`'s interface: every method `C` calls under the same shape
    /// (a node that serves more is fine). Else
    /// [`ClientError::ShapeMismatch`], and nothing else is called.
    pub async fn typed<C: FromRoute<T> + ShapeCheck>(self) -> Result<C, ClientError> {
        let schema = self.schema().await?;
        C::shape_check(self.path(), &schema)?;
        Ok(C::from_route(self.transport, self.route))
    }
}

impl<T: Clone> Clone for Untyped<T> {
    fn clone(&self) -> Self {
        Self { transport: self.transport.clone(), route: self.route.clone() }
    }
}

impl<T: fmt::Debug> fmt::Debug for Untyped<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Untyped").field("transport", &self.transport).field("path", &self.path()).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{MemoryTransport, StreamItem};
    use futures_util::StreamExt;
    use serde_json::json;

    fn data(ct: &str, v: Value) -> StreamItem {
        StreamItem::Data { metadata: None, content_type: Some(ct.into()), content: v }
    }

    #[test]
    fn calls_go_to_the_path_by_name() {
        let t = MemoryTransport::new()
            .script(MethodRef::named("host.dendrite.digest", "summarize"), vec![data("x", json!("a")), data("x", json!("b")), StreamItem::done()])
            .script(MethodRef::named("host.dendrite.digest.inner", "ping"), vec![data("x", json!(1)), StreamItem::done()]);
        let root = Untyped::new(&t, "host");
        let u = root.child("dendrite").into_child("digest");
        assert_eq!(u.path(), "host.dendrite.digest");
        let items: Vec<_> = futures_executor_block(u.call("summarize", json!({"n": 2})).collect::<Vec<_>>());
        assert_eq!(items, vec![Ok(json!("a")), Ok(json!("b"))]);
        assert_eq!(t.params_of(MethodRef::named("host.dendrite.digest", "summarize")), Some(json!({"n": 2})));
        let one = futures_executor_block(u.child("inner").call_one("ping", json!({})));
        assert_eq!(one, Ok(json!(1)));
    }

    #[test]
    fn a_bad_name_sends_nothing() {
        let t = MemoryTransport::new();
        let u = Untyped::new(&t, "host");
        let bad = u.child("Not A Segment");
        let r = futures_executor_block(bad.call_one("ping", json!({})));
        assert!(matches!(r, Err(ClientError::InvalidName { .. })), "{r:?}");
        assert!(t.calls().is_empty());
    }

    fn futures_executor_block<F: std::future::Future>(f: F) -> F::Output {
        futures::executor::block_on(f)
    }
}
