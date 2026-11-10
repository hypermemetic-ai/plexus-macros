//! KEEL-31 compile-fail: a word `#[peer(...)]` / `#[child(...)]` does not take
//! is an error naming it.

use plexus_core::plexus::peer::{Peer, PeerError};

#[derive(Clone)]
struct T;

#[plexus_macros::activation(namespace = "t", crate_path = "plexus_core")]
impl T {
    #[plexus_macros::method]
    async fn get(&self) -> impl futures::stream::Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield String::new(); }
    }

    /// A peer.
    #[plexus_macros::peer(everywhere)]
    async fn far(&self, name: &str) -> Result<Box<dyn Peer>, PeerError> {
        Err(PeerError::NotFound { name: name.into() })
    }
}

fn main() {}
