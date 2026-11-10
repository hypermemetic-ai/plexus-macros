//! KEEL-27: a mount takes no argument or one `name: &str`; a `pub fn`
//! with any other is refused, saying what a mount is.
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

    /// A lookup by number, written `pub`.
    pub fn leaf(&self, n: u32) -> Option<LeafActivation> {
        (n == 1).then_some(LeafActivation)
    }
}

fn main() {}
