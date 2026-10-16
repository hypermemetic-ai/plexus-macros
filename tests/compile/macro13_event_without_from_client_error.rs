//! MACRO-13: the client's implementation of the trait yields a client
//! failure as the event type's own; an event type that cannot say so is
//! named.

use futures::Stream;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum EchoEvent {
    Said(String),
}

#[derive(Clone)]
pub struct EchoActivation;

#[plexus_macros::activation]
impl EchoActivation {
    pub async fn say(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        futures::stream::iter([EchoEvent::Said(text)])
    }
}

fn main() {}
