//! MACRO-13 fixture — no `plexus-client` in the manifest, so no client: the
//! crate builds (a client would not resolve here), and the trait still
//! stands, its streams futures' `BoxStream`.

use futures::Stream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum EchoEvent {
    Said(String),
}

#[derive(Clone, Default)]
pub struct EchoActivation;

/// Echo things back.
#[plexus_macros::activation]
impl EchoActivation {
    /// Say a thing back.
    pub async fn say(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        futures::stream::iter([EchoEvent::Said(text)])
    }
}
