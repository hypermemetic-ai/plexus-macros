//! NEST-14: an opaque child's accessor is untyped — it calls by name, and
//! has none of the child's typed methods until made typed.

use futures::Stream;
use plexus_client::MemoryTransport;

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
    /// Opaque.
    #[plexus_macros::child(opaque)]
    fn raw(&self) -> Leaf {
        Leaf
    }
}

fn main() {
    let t = MemoryTransport::new();
    let _ = TreeClient::new(&t).raw().ping();
}
