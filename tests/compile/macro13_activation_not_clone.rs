//! MACRO-13: an activation that is not `Clone` is told so, by name — its
//! trait is implemented by cloning it into each stream. RETURNS-47: a trait
//! only when named, so it is named here.

use futures::Stream;

pub struct PingActivation;

#[plexus_macros::activation(trait = "Ping")]
impl PingActivation {
    pub async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::empty()
    }
}

fn main() {}
