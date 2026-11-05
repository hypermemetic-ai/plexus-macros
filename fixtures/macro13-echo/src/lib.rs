//! MACRO-13 fixture — the `server` feature default.
//!
//! | Build | Gets |
//! |---|---|
//! | default (`server`, `host`) | the three activations, their clients, the named trait `Echo` and the contract `Promise` |
//! | `--no-default-features --features server` | `EchoActivation`'s server half; `HostActivation`'s is gated on `host` |
//! | `--no-default-features` | the traits and the clients, no plexus-core in the tree |
//!
//! RETURNS-47: a trait only when named — `Echo` is (`trait = "Echo"`),
//! `Host` is not and has none; a contract (`Promise`) is its trait. The
//! clients implement no trait.
//!
//! Neither attribute says `server_cfg` for the `server` gate: the crate's
//! manifest declares a `server` feature, so the macro gates on it.

#[cfg(feature = "server")]
use futures::Stream;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum EchoEvent {
    Said { text: String },
    Error { message: String },
}

/// The author gates their own struct; the macro gates its output.
#[cfg(feature = "server")]
#[derive(Clone, Default)]
pub struct EchoActivation;

/// Echo things back.
#[plexus_macros::activation(trait = "Echo")]
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

/// The client, in every build.
pub fn reach_it<T: plexus_client::Transport>(transport: T) -> EchoClient<T> {
    EchoClient::new(transport)
}

/// `HostClient`, in every build.
pub fn reach_the_host<T: plexus_client::Transport>(transport: T) -> HostClient<T> {
    HostClient::new(transport)
}

/// `PromiseClient`, in every build.
pub fn reach_the_promise<T: plexus_client::Transport>(transport: T) -> PromiseClient<T> {
    PromiseClient::new(transport)
}

/// The named trait and the contract's, in every build: what an
/// implementation outside the server half is written against.
pub fn the_traits(_: &dyn Echo, _: &dyn Promise) {}

#[cfg(all(test, feature = "host"))]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use plexus_core::plexus::{Activation, InProcessTransport};

    #[tokio::test]
    async fn both_halves_with_default_features() {
        assert_eq!(EchoActivation.namespace(), "echo");
        assert_eq!(HostActivation.namespace(), "host");
        let echo = reach_it(InProcessTransport::new(EchoActivation));
        let got: Vec<EchoEvent> = echo.say("hi".into()).try_collect().await.unwrap();
        assert_eq!(got, vec![EchoEvent::Said { text: "hi".into() }]);
        let host = reach_the_host(InProcessTransport::new(HostActivation));
        let got: Vec<EchoEvent> = host.say("hi".into()).try_collect().await.unwrap();
        assert_eq!(got, vec![EchoEvent::Said { text: "hi".into() }]);
        // The named trait, implemented by the activation.
        let through: Vec<EchoEvent> = futures::StreamExt::collect(Echo::say(&EchoActivation, "hi".into())).await;
        assert_eq!(through, vec![EchoEvent::Said { text: "hi".into() }]);
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
        let got: Vec<EchoEvent> = promise.keep("hi".into()).try_collect().await.unwrap();
        assert_eq!(got, vec![EchoEvent::Said { text: "hi".into() }]);
    }
}
