//! KEEL-27: a slot returns `Option<A>`; anything else is refused.
use async_stream::stream;
use futures::stream::Stream;

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

/// Not an activation: a plain value.
#[derive(Clone)]
pub struct User;

/// A shelf.
#[derive(Clone)]
pub struct ShelfActivation;

#[plexus_macros::activation(served = written)]
impl ShelfActivation {
    /// Ping.
    pub async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// A slot that answers a leaf, not an `Option` of one.
    pub fn leaf(&self, _name: &str) -> LeafActivation {
        LeafActivation
    }
}

fn main() {}
