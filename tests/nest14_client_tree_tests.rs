//! NEST-14 · the activation's client all the way down: every static child,
//! dynamic child, peer slot and opaque child gets an accessor, so one
//! client reaches any node of the tree.
//!
//! | child | accessor |
//! | -- | -- |
//! | static `#[child]` | its typed client (RETURNS-46, unchanged) |
//! | dynamic `#[child]` | its typed client by name (unchanged) |
//! | typed `#[peer]` slot | `async fn slot(name) -> Result<ForgeClient<PeerTransport<T>>, PeerError>`, stamp-checked |
//! | untyped `#[peer]` slot | `fn slot(name) -> Untyped<&T>` |
//! | `#[child(opaque)]` | `fn x() -> Untyped<&T>` / `fn x(name) -> Untyped<&T>` |
//!
//! An `Untyped` calls by name, reaches below by name, answers its schema,
//! and becomes a typed client once its schema says it serves that client's
//! interface (`ShapeMismatch` else).

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_client::{ClientError, MemoryTransport, Untyped};
use plexus_core::plexus::peer::{Peer, PeerError, PeerTransport};
use plexus_core::plexus::{HubTransport, InProcessTransport};
use plexus_core::DynamicHub;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

// ------------------------------------------------------------------ the contract

pub mod forge {
    use super::*;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    pub struct Repo {
        pub name: String,
    }

    /// Why a forge said no — or could not be reached.
    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
    #[serde(tag = "type", rename_all = "snake_case")]
    pub enum ForgeError {
        Denied { why: String },
        Peer { error: PeerError },
    }

    impl From<PeerError> for ForgeError {
        fn from(error: PeerError) -> Self {
            ForgeError::Peer { error }
        }
    }

    /// A forge: repositories.
    #[plexus_macros::activation]
    impl ForgeActivation {
        /// Every repository.
        #[plexus_macros::method]
        pub async fn repos(&self) -> Result<Vec<Repo>, ForgeError>;
    }
}

use forge::{Forge, ForgeClient, ForgeError, ForgePeer, Repo};

#[derive(Clone)]
struct GitHub;

impl Forge for GitHub {
    fn repos(&self) -> plexus_client::BoxFuture<'static, Result<Vec<Repo>, ForgeError>> {
        Box::pin(async { Ok(vec![Repo { name: "plexus-core".into() }]) })
    }
}

pub mod rebuilt {
    use super::*;

    /// A rebuilt backend whose `repos` changed.
    #[derive(Clone)]
    pub struct RebuiltActivation;

    /// A forge, rebuilt.
    #[plexus_macros::activation(namespace = "forge")]
    impl RebuiltActivation {
        /// Every repository's name.
        #[plexus_macros::method]
        pub async fn repos(&self) -> Result<Vec<String>, ForgeError> {
            Ok(vec!["rebuilt".into()])
        }
    }
}

// ------------------------------------------------------------------ the tree

/// Says hello.
#[derive(Clone)]
pub struct Greeter;

#[plexus_macros::activation]
impl Greeter {
    /// Hello.
    pub async fn hello(&self, name: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield format!("hello, {name}"); }
    }
}

/// Counts.
#[derive(Clone)]
pub struct Stats;

#[plexus_macros::activation]
impl Stats {
    /// How many.
    pub async fn count(&self) -> impl Stream<Item = u32> + Send + 'static {
        stream! { yield 3; }
    }
}

type Make = Arc<dyn Fn() -> Box<dyn ForgePeer> + Send + Sync>;

#[derive(Clone)]
pub struct Host {
    wrong: Make,
}

#[plexus_macros::activation]
impl Host {
    /// The integrations' names.
    pub async fn integrations(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "github".to_string(); }
    }

    /// Stats: a static child with a client.
    #[plexus_macros::child]
    fn stats(&self) -> Stats {
        Stats
    }

    /// A child the host does not type.
    #[plexus_macros::child(opaque)]
    fn raw(&self) -> Greeter {
        Greeter
    }

    /// A typed peer: a forge.
    #[plexus_macros::peer(list = integrations)]
    async fn integration(&self, name: &str) -> Result<Box<dyn ForgePeer>, PeerError> {
        match name {
            "github" => Ok(Box::new(GitHub.server())),
            "wrong" => Ok((self.wrong)()),
            _ => Err(PeerError::NotFound { name: name.into() }),
        }
    }

    /// An untyped peer: anything.
    #[plexus_macros::peer]
    async fn dendrite(&self, name: &str) -> Result<Box<dyn Peer>, PeerError> {
        match name {
            "greeter" => Ok(Box::new(Greeter)),
            _ => Err(PeerError::NotFound { name: name.into() }),
        }
    }
}

/// A shelf of greeters by name, not typed.
#[derive(Clone)]
pub struct Shelf;

#[plexus_macros::activation]
impl Shelf {
    /// Size.
    pub async fn size(&self) -> impl Stream<Item = u32> + Send + 'static {
        stream! { yield 1; }
    }

    /// A greeter by name.
    #[plexus_macros::child(opaque)]
    fn item(&self, name: &str) -> Option<Greeter> {
        (name == "a").then_some(Greeter)
    }
}

async fn served() -> HubTransport {
    let t = PeerTransport::new(InProcessTransport::new(rebuilt::RebuiltActivation));
    t.check("forge", ForgeClient::<()>::stamp()).await.unwrap();
    let wrong = ForgeClient::at(t, "forge");
    let host = Host { wrong: Arc::new(move || Box::new(wrong.clone()) as Box<dyn ForgePeer>) };
    HubTransport::anonymous(Arc::new(DynamicHub::new("svc").register(host).register(Shelf)))
}

// ------------------------------------------------------------------ acceptance

#[test]
fn every_kind_of_child_has_an_accessor_at_its_path() {
    let t = MemoryTransport::new();
    let host = HostClient::new(&t);
    assert_eq!(host.stats().path(), "host.stats");
    let raw: Untyped<&&MemoryTransport> = host.raw();
    assert_eq!(raw.path(), "host.raw");
    assert_eq!(host.dendrite("greeter").path(), "host.dendrite.greeter");
    assert_eq!(host.dendrite("greeter").child("deeper").path(), "host.dendrite.greeter.deeper");
    assert_eq!(ShelfClient::new(&t).item("a").path(), "shelf.a");
    // Mounted elsewhere, every path follows.
    let mounted = HostClient::at(&t, "outer.host");
    assert_eq!(mounted.dendrite("g").path(), "outer.host.dendrite.g");
}

#[tokio::test]
async fn an_untyped_peer_is_called_by_name() {
    let t = served().await;
    let host = HostClient::new(t);
    let got: Vec<_> = host.dendrite("greeter").call("hello", json!({ "name": "world" })).collect().await;
    assert_eq!(got, vec![Ok(json!("hello, world"))]);
    let got: Vec<_> = host.raw().call("hello", json!({ "name": "raw" })).collect().await;
    assert_eq!(got, vec![Ok(json!("hello, raw"))]);
    let got: Vec<_> = ShelfClient::new(host.transport().clone()).item("a").call("hello", json!({ "name": "a" })).collect().await;
    assert_eq!(got, vec![Ok(json!("hello, a"))]);
    // The typed static child, as before.
    assert_eq!(host.stats().count().collect::<Vec<_>>().await, vec![Ok(3)]);
}

#[tokio::test]
async fn an_untyped_peer_becomes_typed_once_its_schema_says_so() {
    let t = served().await;
    let host = HostClient::new(t);
    let greeter: GreeterClient<_> = host.dendrite("greeter").typed().await.expect("a greeter");
    assert_eq!(greeter.path(), "host.dendrite.greeter");
    assert_eq!(greeter.hello("typed".into()).collect::<Vec<_>>().await, vec![Ok("hello, typed".to_string())]);
    // Not a forge: refused, by shape.
    match host.dendrite("greeter").typed::<StatsClient<_>>().await {
        Err(ClientError::ShapeMismatch { path, methods, .. }) => {
            assert_eq!(path, "host.dendrite.greeter");
            assert_eq!(methods, vec!["count".to_string()]);
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_typed_slot_is_its_contracts_client_checked_by_stamp() {
    let t = served().await;
    let host = HostClient::new(t);
    let gh = host.integration("github").await.expect("a forge");
    assert_eq!(Forge::repos(&gh).await, Ok(vec![Repo { name: "plexus-core".into() }]));
    // A peer whose `repos` changed is refused.
    match host.integration("wrong").await {
        Err(PeerError::ShapeMismatch { methods, .. }) => assert_eq!(methods, vec!["repos".to_string()]),
        Err(other) => panic!("{other:?}"),
        Ok(_) => panic!("admitted a peer of another shape"),
    }
    // A name that is not a segment is not found, and nothing is sent.
    assert!(matches!(host.integration("Not Here").await, Err(PeerError::NotFound { .. })));
}
