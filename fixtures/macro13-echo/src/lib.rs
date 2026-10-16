//! MACRO-13 fixture — the `server` feature default.
//!
//! | Build | Gets |
//! |---|---|
//! | default (`server`, `host`) | the three activations, their traits and clients |
//! | `--no-default-features --features server` | `EchoActivation`'s server half; `HostActivation`'s is gated on `host` |
//! | `--no-default-features` | the traits and the clients, no plexus-core in the tree |
//!
//! Neither attribute says `server_cfg` for the `server` gate: the crate's
//! manifest declares a `server` feature, so the macro gates on it.

#[cfg(feature = "server")]
use futures::Stream;
use plexus_client::{ClientError, FromClientError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EchoEvent {
    Said { text: String },
    Error { message: String },
}

impl FromClientError for EchoEvent {
    fn from_client_error(error: ClientError) -> Self {
        EchoEvent::Error { message: error.to_string() }
    }
}

/// The author gates their own struct; the macro gates its output.
#[cfg(feature = "server")]
#[derive(Clone, Default)]
pub struct EchoActivation;

/// Echo things back.
#[plexus_macros::activation]
impl EchoActivation {
    /// Say a thing back.
    pub async fn say(&self, /** what to say */ text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        futures::stream::iter([EchoEvent::Said { text }])
    }
}

/// Gated on `host`, not `server`: with `--features server` alone this struct
/// is absent, and the build passes only if the macro's server half is too.
#[cfg(feature = "host")]
#[derive(Clone, Default)]
pub struct HostActivation;

/// Echo, hosted.
#[plexus_macros::activation(server_cfg = "feature = \"host\"")]
impl HostActivation {
    /// Say a thing back.
    pub async fn say(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static {
        futures::stream::iter([EchoEvent::Said { text }])
    }
}

/// A contract: its struct and `Promise::server` are the server half, gated
/// on `server` with everything else; the trait and `PromiseClient` are not.
#[plexus_macros::activation]
impl PromiseActivation {
    /// Keep it.
    pub async fn keep(&self, text: String) -> impl Stream<Item = EchoEvent> + Send + 'static;
}

/// The traits and the clients, in every build.
pub fn reach_it_through_the_trait<T>(transport: T) -> Box<dyn Echo>
where
    T: plexus_client::Transport + Clone + Send + Sync + 'static,
{
    Box::new(EchoClient::new(transport))
}

/// `Host` and `HostClient`, in every build.
pub fn reach_the_host<T>(transport: T) -> Box<dyn Host>
where
    T: plexus_client::Transport + Clone + Send + Sync + 'static,
{
    Box::new(HostClient::new(transport))
}

/// `Promise` and `PromiseClient`, in every build.
pub fn reach_the_promise<T>(transport: T) -> Box<dyn Promise>
where
    T: plexus_client::Transport + Clone + Send + Sync + 'static,
{
    Box::new(PromiseClient::new(transport))
}

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;
    use futures::StreamExt;
    use plexus_core::plexus::{Activation, InProcessTransport};

    #[tokio::test]
    async fn both_halves_with_default_features() {
        assert_eq!(EchoActivation.namespace(), "echo");
        assert_eq!(HostActivation.namespace(), "host");
        let echo = reach_it_through_the_trait(InProcessTransport::new(EchoActivation));
        let got: Vec<EchoEvent> = echo.say("hi".into()).collect().await;
        assert_eq!(got, vec![EchoEvent::Said { text: "hi".into() }]);
        let host = reach_the_host(InProcessTransport::new(HostActivation));
        let got: Vec<EchoEvent> = host.say("hi".into()).collect().await;
        assert_eq!(got, vec![EchoEvent::Said { text: "hi".into() }]);
    }

    struct Kept;
    impl Promise for Kept {
        fn keep(&self, text: String) -> plexus_client::BoxStream<'static, EchoEvent> {
            Box::pin(futures::stream::iter([EchoEvent::Said { text }]))
        }
    }

    #[tokio::test]
    async fn a_contract_with_default_features() {
        let served = Kept.server();
        assert_eq!(served.namespace(), "promise");
        let promise = reach_the_promise(InProcessTransport::new(served));
        let got: Vec<EchoEvent> = promise.keep("hi".into()).collect().await;
        assert_eq!(got, vec![EchoEvent::Said { text: "hi".into() }]);
    }
}
