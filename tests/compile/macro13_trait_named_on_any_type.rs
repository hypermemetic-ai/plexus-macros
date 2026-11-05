//! MACRO-13: `trait = "Echo"` names the trait and the client, whatever the
//! type is called — here `impl Server`. The namespace is the trait's name in
//! snake_case. RETURNS-47: the activation implements the trait; the client
//! implements none, and answers `Result`s.

use futures::Stream;
use plexus_client::{BoxStream, MemoryTransport};
use plexus_core::plexus::Activation;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
pub enum EchoEvent {
    Said(String),
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
    let _: fn(&dyn Echo, String) -> BoxStream<'static, EchoEvent> = |e, t| e.say(t);
    assert_eq!(Server.namespace(), "echo");
    assert_eq!(EchoClient::<MemoryTransport>::NAMESPACE, "echo");
}
