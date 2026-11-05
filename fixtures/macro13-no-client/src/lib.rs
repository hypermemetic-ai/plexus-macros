//! MACRO-13 fixture — no `plexus-client` in the manifest, so no client: the
//! crate builds (a client would not resolve here), and a named trait still
//! stands (RETURNS-47: `trait = "Echo"`, a trait only when named), its
//! streams futures' `BoxStream`.

use futures::Stream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
pub enum EchoEvent {
    Said(String),
}

#[derive(Clone, Default)]
pub struct EchoActivation;

/// Echo things back.
#[plexus_macros::activation(trait = "Echo")]
impl EchoActivation {
    /// Say a thing back.
    pub async fn say(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        futures::stream::iter([EchoEvent::Said(text)])
    }
}
