//! KEEL-27: a generic block under `served = written` — the compiler's check
//! of each inferred mount sits in the impl, where its generics are in scope.
//! (A mount typed by the impl's own parameter has no client to name, with a
//! marker or without: the client is named from the return type.)

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::ChildRouter;

/// A leaf.
#[derive(Clone)]
pub struct LeafActivation;

#[plexus_macros::activation]
impl LeafActivation {
    /// Ping.
    pub async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

/// A shelf over any mount.
#[derive(Clone)]
pub struct ShelfActivation<M: ChildRouter + Clone + Send + Sync + 'static> {
    inner: M,
}

#[plexus_macros::activation(served = written)]
impl<M: ChildRouter + Clone + Send + Sync + 'static> ShelfActivation<M> {
    /// Ping.
    pub async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// The leaf.
    pub fn first(&self) -> LeafActivation {
        let _ = &self.inner;
        LeafActivation
    }

    /// A leaf by name.
    pub fn leaf(&self, name: &str) -> Option<LeafActivation> {
        (name == "a").then_some(LeafActivation)
    }

    /// A helper.
    fn helper(&self) -> u32 {
        1
    }

    /// A constructor.
    pub fn new(inner: M) -> Self {
        let s = ShelfActivation { inner };
        let _ = s.helper();
        s
    }
}

fn main() {
    let _ = ShelfActivation::new(LeafActivation);
}
