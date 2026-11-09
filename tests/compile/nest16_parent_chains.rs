//! NEST-16: chains on `parent` build; `uses` lets it be passed on.
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::Parent;

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

#[derive(Clone)]
pub struct Restock;

/// Knows its parent.
#[plexus_macros::activation(parent = ShopClient)]
impl Restock {
    /// Items, in a closure and a macro.
    pub async fn low(&self, parent: Parent<ShopClient>) -> impl Stream<Item = String> + Send + 'static {
        let n = parent.items().count().await;
        println!("{} {}", n, parent.items().count().await);
        futures::stream::iter([n.to_string()])
    }
    /// Passed on, said by hand.
    #[plexus_macros::method(uses("shop.items"))]
    pub async fn passed(&self, parent: Parent<ShopClient>) -> impl Stream<Item = usize> + Send + 'static {
        let p = parent;
        futures::stream::iter([p.items().count().await])
    }
}

fn main() {}
