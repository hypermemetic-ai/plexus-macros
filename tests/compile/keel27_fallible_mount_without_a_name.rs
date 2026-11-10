//! KEEL-27: `Result<Box<dyn I>, PeerError>` is a fallible mount, which takes
//! a name.
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

    /// A fallible mount with no name.
    pub async fn remote(&self) -> Result<Box<dyn plexus_core::plexus::peer::Peer>, plexus_core::plexus::peer::PeerError> {
        Err(plexus_core::plexus::peer::PeerError::Timeout)
    }
}

fn main() {}
