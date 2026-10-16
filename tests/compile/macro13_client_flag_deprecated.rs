//! MACRO-13: under the name rule the client is emitted whenever the crate
//! depends on plexus-client, so `client` says nothing: it is accepted, with a
//! deprecation warning naming MACRO-13 (an error here, under `deny`, so the
//! fixture can show it).
#![deny(deprecated)]

use futures::Stream;

#[derive(Clone)]
pub struct EchoActivation;

#[plexus_macros::activation(client)]
impl EchoActivation {
    pub async fn say(&self, text: String) -> impl Stream<Item = String> + Send + 'static {
        futures::stream::iter([text])
    }
}

fn main() {
    let _ = EchoClient::new(plexus_client::MemoryTransport::new());
}
