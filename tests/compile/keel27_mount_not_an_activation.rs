//! KEEL-27: under `served = written` a `pub fn` on `&self` is a mount; one
//! returning something that is not an activation is refused at its return
//! type, the message naming the fn and saying what a mount returns.
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

    /// A helper written `pub`: served as a mount, and refused.
    pub fn user(&self) -> User {
        User
    }
}

fn main() {}
