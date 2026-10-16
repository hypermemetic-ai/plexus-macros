//! MACRO-13 (`always`): the trait is always emitted, so a bare `trait` asks
//! for nothing; `trait = "Name"` names it.

use futures::Stream;

#[derive(Clone)]
pub struct EchoActivation;

#[plexus_macros::activation(trait)]
impl EchoActivation {
    pub async fn say(&self, text: String) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::iter([text])
    }
}

fn main() {}
