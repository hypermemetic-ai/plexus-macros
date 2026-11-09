//! NEST-16: a `Parent` with no `parent = …` on the activation.
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

/// Names no parent.
#[plexus_macros::activation]
impl Restock {
    /// Items.
    pub async fn low(&self, parent: Parent<ShopClient>) -> impl Stream<Item = usize> + Send + 'static {
        futures::stream::iter([parent.items().count().await])
    }
}

fn main() {}
