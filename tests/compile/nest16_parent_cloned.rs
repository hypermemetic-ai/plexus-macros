//! NEST-16: `parent` leaving a call chain, with no `uses`, is refused here.
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Parent, ParentTransport};

pub mod shop {
    use futures::Stream;
    #[derive(Clone)]
    pub struct Shop;
    /// The shop.
    #[plexus_macros::activation]
    impl Shop {
        /// Items.
        pub async fn items(&self) -> impl Stream<Item = String> + Send + 'static {
            futures::stream::iter(["flour".to_string()])
        }
    }
}
use shop::ShopClient;

#[allow(dead_code)]
async fn count(p: &Parent<ShopClient<ParentTransport>>) -> usize {
    p.items().count().await
}

#[derive(Clone)]
pub struct Restock;

/// Knows its parent.
#[plexus_macros::activation(parent = ShopClient)]
impl Restock {
    /// Items.
    pub async fn low(&self, parent: Parent<ShopClient>) -> impl Stream<Item = usize> + Send + 'static {
        let n = parent.clone().items().count().await;
        futures::stream::iter([n])
    }
}

fn main() {}
