//! `#[child(opaque)]` (RETURNS-46 review): a dynamic child whose type has no
//! client gets no accessor and no variant, and is routed as before.
//!
//! [`SessionChild`] is plexus-substrate's `claudecode` shape: a legacy name
//! (no `…Activation`, no `client` flag), so no `SessionChildClient`.
//! [`RawRouter`] is a hand-written `ChildRouter`, no macro at all.

use super::{ticks, N};
use async_trait::async_trait;
use futures::Stream;
use plexus_core::plexus::{wrap_stream, ChildRouter, PlexusError, PlexusStream};
use serde_json::Value;

/// substrate's `ClaudeCode`: a parent whose dynamic child has no client.
#[derive(Debug, Clone, Default)]
pub struct HostActivation;

#[plexus_macros::activation(namespace = "host", description = "host", crate_path = "plexus_core")]
impl HostActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }
    /// a session by id — as substrate's `claudecode.session`
    #[plexus_macros::child(opaque)]
    pub fn session(&self, id: &str) -> Option<SessionChild> {
        Some(SessionChild { id: id.to_string() })
    }
}

/// substrate's `SessionChild`: an activation with no client.
#[derive(Debug, Clone, Default)]
pub struct SessionChild {
    id: String,
}

#[allow(deprecated)]
#[plexus_macros::activation(namespace = "session", description = "a session", crate_path = "plexus_core")]
impl SessionChild {
    /// the session's id, n times
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        let len = self.id.len() as u32;
        async_stream::stream! { for i in 0..n { yield N { n: len * 100 + i }; } }
    }
}

/// A parent whose dynamic child is a hand-written `ChildRouter`.
#[derive(Debug, Clone, Default)]
pub struct RawHostActivation;

#[plexus_macros::activation(namespace = "raw_host", description = "raw host", crate_path = "plexus_core")]
impl RawHostActivation {
    /// ticks
    #[plexus_macros::method(params(n = "count"))]
    pub async fn tick(&self, n: u32) -> impl Stream<Item = N> + Send + 'static {
        ticks(n)
    }
    /// a raw router by name
    #[plexus_macros::child(opaque)]
    pub fn raw(&self, name: &str) -> Option<RawRouter> {
        Some(RawRouter { name: name.to_string() })
    }
}

/// A `ChildRouter` written by hand: `hello` answers its name.
#[derive(Debug, Clone)]
pub struct RawRouter {
    name: String,
}

#[async_trait]
impl ChildRouter for RawRouter {
    fn router_namespace(&self) -> &str {
        "raw"
    }

    async fn router_call(
        &self,
        method: &str,
        _params: Value,
        _auth: Option<&plexus_core::plexus::AuthContext>,
        _raw: Option<&plexus_core::request::RawRequestContext>,
    ) -> Result<PlexusStream, PlexusError> {
        match method {
            "hello" => Ok(wrap_stream(
                futures::stream::iter(vec![serde_json::json!({ "hello": self.name })]),
                "raw.hello",
                vec!["raw".into()],
            )),
            other => Err(PlexusError::MethodNotFound { activation: "raw".into(), method: other.into() }),
        }
    }

    async fn get_child(&self, _name: &str) -> Option<Box<dyn ChildRouter>> {
        None
    }
}
