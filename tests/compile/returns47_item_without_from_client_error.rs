//! RETURNS-47: the client implements no trait, so nothing asks an item type
//! for `FromClientError` — an event enum, `u64`, `bool` and `Vec<T>` alike.
//! (MACRO-13 and RETURNS-36 refused each.)

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

    #[plexus_macros::method]
    pub async fn total(&self) -> u64 {
        3
    }

    #[plexus_macros::method]
    pub async fn up(&self) -> bool {
        true
    }

    #[plexus_macros::method]
    pub async fn names(&self) -> Vec<String> {
        vec![]
    }
}

fn main() {}
