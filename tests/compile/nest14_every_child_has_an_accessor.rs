//! NEST-14: every row of the table has an accessor, by its type — a static
//! and a dynamic child their typed clients, an opaque child and an untyped
//! peer slot an `Untyped`, a typed peer slot its contract's client.

use futures::Stream;
use plexus_client::{MemoryTransport, Named, Root, Static, Untyped};
use plexus_core::plexus::peer::{Peer, PeerError, PeerTransport};
use std::sync::Arc;

pub mod forge {
    use plexus_core::plexus::peer::PeerError;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
    pub struct ForgeError(pub String);
    impl From<PeerError> for ForgeError {
        fn from(e: PeerError) -> Self {
            ForgeError(e.to_string())
        }
    }

    /// A forge.
    #[plexus_macros::activation]
    impl ForgeActivation {
        /// Repositories.
        #[plexus_macros::method]
        pub async fn repos(&self) -> Result<Vec<String>, ForgeError>;
    }
}

#[derive(Clone)]
pub struct Leaf;

#[plexus_macros::activation]
impl Leaf {
    /// Ping.
    pub async fn ping(&self) -> impl Stream<Item = u32> + Send + 'static {
        futures::stream::iter([1])
    }
}

#[derive(Clone)]
pub struct Tree;

#[plexus_macros::activation]
impl Tree {
    /// Size.
    pub async fn size(&self) -> impl Stream<Item = u32> + Send + 'static {
        futures::stream::iter([1])
    }
    /// Static.
    #[plexus_macros::child]
    fn branch(&self) -> Leaf {
        Leaf
    }
    /// Dynamic.
    #[plexus_macros::child]
    fn item(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }
    /// Opaque.
    #[plexus_macros::child(opaque)]
    fn raw(&self) -> Leaf {
        Leaf
    }
    /// Typed slot.
    #[plexus_macros::peer]
    async fn forge(&self, _name: &str) -> Result<Box<dyn forge::ForgePeer>, PeerError> {
        Err(PeerError::Timeout)
    }
    /// Untyped slot.
    #[plexus_macros::peer]
    async fn any(&self, _name: &str) -> Result<Box<dyn Peer>, PeerError> {
        Err(PeerError::Timeout)
    }
}

#[allow(dead_code)]
async fn reach(t: Arc<MemoryTransport>) {
    let tree = TreeClient::new(t.clone());
    let _: LeafClient<&Arc<MemoryTransport>, Static<Root, _>> = tree.branch();
    let _: LeafClient<&Arc<MemoryTransport>, Named<Root, _, TreeEvent>> = tree.item("a");
    let _: Untyped<&Arc<MemoryTransport>> = tree.raw();
    let _: Untyped<&Arc<MemoryTransport>> = tree.any("x");
    let _: Result<forge::ForgeClient<PeerTransport<Arc<MemoryTransport>>>, PeerError> = tree.forge("x").await;
    let _: Result<LeafClient<&Arc<MemoryTransport>>, plexus_client::ClientError> = tree.any("x").typed().await;
}

fn main() {}
