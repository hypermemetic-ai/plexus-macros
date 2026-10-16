//! MACRO-13: an activation that is not `Clone` is told so, by name — its
//! trait is implemented by cloning it into each stream.

use futures::Stream;

pub struct PingActivation;

#[plexus_macros::activation]
impl PingActivation {
    pub async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {}
