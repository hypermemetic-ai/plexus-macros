//! RETURNS-47: a trait only when named — a bare `trait` names none;
//! `trait = "Name"` does.

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
