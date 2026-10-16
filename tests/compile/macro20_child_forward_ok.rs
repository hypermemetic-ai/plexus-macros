//! MACRO-20: every `#[child(forward = "...")]` spelling compiles, on a
//! static and a dynamic child, and a static child that is a hand-written
//! `ChildRouter` (no `Activation`, no schema) still compiles beside them.

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::{AuthContext, ChildRouter, PlexusError, PlexusStream};

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(namespace = "leaf", version = "1.0.0", crate_path = "plexus_core")]
impl Leaf {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

/// A hand-written router: describes nothing.
#[derive(Clone)]
struct Manual;

#[async_trait::async_trait]
impl ChildRouter for Manual {
    fn router_namespace(&self) -> &str {
        "manual"
    }
    async fn router_call(
        &self,
        method: &str,
        _params: serde_json::Value,
        _auth: Option<&AuthContext>,
        _raw_ctx: Option<&plexus_core::request::RawRequestContext>,
    ) -> Result<PlexusStream, PlexusError> {
        Err(PlexusError::MethodNotFound { activation: "manual".into(), method: method.into() })
    }
    async fn get_child(&self, _name: &str) -> Option<Box<dyn ChildRouter>> {
        None
    }
}

#[derive(Clone)]
struct Hub;

#[plexus_macros::activation(namespace = "hub", version = "1.0.0", crate_path = "plexus_core")]
impl Hub {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Default: same_service.
    #[plexus_macros::child]
    fn plain(&self) -> Leaf {
        Leaf
    }

    #[plexus_macros::child(forward = "same_service")]
    fn same(&self) -> Leaf {
        Leaf
    }

    #[plexus_macros::child(forward = "identity_only")]
    fn deputy(&self) -> Leaf {
        Leaf
    }

    #[plexus_macros::child(forward = "pass_through")]
    fn through(&self) -> Leaf {
        Leaf
    }

    #[plexus_macros::child(forward = "anonymous")]
    fn nobody(&self) -> Leaf {
        Leaf
    }

    #[plexus_macros::child]
    fn manual(&self) -> Manual {
        Manual
    }

    #[plexus_macros::child(forward = "pass_through")]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }
}

fn main() {
    let _ = Hub.forward_policy_for("plain");
}
