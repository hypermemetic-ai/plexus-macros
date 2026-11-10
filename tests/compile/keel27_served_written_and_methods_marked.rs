//! KEEL-27: `served = written` and `methods = marked` are two rules for
//! the unmarked fns; both is refused.
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

#[plexus_macros::activation(served = written, methods = marked)]
impl ShelfActivation {
    /// Ping.
    pub async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }


}

fn main() {}
