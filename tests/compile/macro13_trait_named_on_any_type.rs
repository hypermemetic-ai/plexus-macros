//! MACRO-13: `trait = "Echo"` names the trait and the client, whatever the
//! type is called — here `impl Server`. The namespace is the trait's name in
//! snake_case.

use futures::Stream;
use plexus_client::{BoxStream, ClientError, FromClientError, MemoryTransport};
use plexus_core::plexus::Activation;
use std::sync::Arc;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum EchoEvent {
    Said(String),
    Error(String),
}

impl FromClientError for EchoEvent {
    fn from_client_error(error: ClientError) -> Self {
        EchoEvent::Error(error.to_string())
    }
}

#[derive(Clone)]
pub struct Server;

#[plexus_macros::activation(trait = "Echo")]
impl Server {
    /// Say.
    pub async fn say(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        futures::stream::iter([EchoEvent::Said(text)])
    }
}

fn main() {
    fn is_echo<T: Echo>() {}
    is_echo::<Server>();
    is_echo::<EchoClient<Arc<MemoryTransport>>>();
    let _: fn(&dyn Echo, String) -> BoxStream<'static, EchoEvent> = |e, t| e.say(t);
    assert_eq!(Server.namespace(), "echo");
    assert_eq!(EchoClient::<MemoryTransport>::NAMESPACE, "echo");
}
