//! MACRO-13: a type not named `…Activation`, with no `trait = "…"`, cannot
//! have its trait, client or namespace named.

use futures::Stream;

#[derive(Clone)]
pub struct Driver;

#[plexus_macros::activation]
impl Driver {
    pub async fn describe(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {}
