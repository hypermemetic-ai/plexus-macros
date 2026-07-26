//! CHILD-8 acceptance tests: `#[plexus_macros::child]` methods alone should
//! make an activation wire-equivalent to a legacy `hub` + hand-written
//! `ChildRouter` + hand-written `plugin_children` triad.
//!
//! Observables verified:
//!   - `plugin_schema().is_hub()` → `true`
//!   - `plugin_schema().children.len()` ≥ 1 (one per static `#[child]`)
//!   - Nested routing: `Activation::call(self, "child.method", ...)` returns
//!     the child's response (not `MethodNotFound`)
//!   - `ChildRouter::get_child("child_name")` returns `Some` (CHILD-3 regression)
//!   - `ChildRouter::list_children()` with `list = "..."` opt-in returns
//!     `Some(stream)` (CHILD-4 regression)

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Activation, ChildRouter};

// ---------------------------------------------------------------------------
// Leaf child used by the test activations below.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Mercury;

#[plexus_macros::activation(
    namespace = "mercury",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Mercury {
    /// Return a greeting.
    #[plexus_macros::method]
    async fn info(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "mercury-info".to_string(); }
    }
}

#[derive(Clone)]
struct Venus;

#[plexus_macros::activation(
    namespace = "venus",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Venus {
    /// Return a greeting.
    #[plexus_macros::method]
    async fn info(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "venus-info".to_string(); }
    }
}

// ---------------------------------------------------------------------------
// The test subject: a `#[child]`-only activation.
// No `hub` flag. No hand-written `ChildRouter`. No hand-written `plugin_children`.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct InferredHub {
    mercury: Mercury,
    venus: Venus,
}

#[plexus_macros::activation(
    namespace = "inferred_hub",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl InferredHub {
    /// Health check so the activation is not method-empty.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// The Mercury child.
    #[plexus_macros::child]
    fn mercury(&self) -> Mercury {
        self.mercury.clone()
    }

    /// The Venus child.
    #[plexus_macros::child]
    fn venus(&self) -> Venus {
        self.venus.clone()
    }
}

fn make_hub() -> InferredHub {
    InferredHub {
        mercury: Mercury,
        venus: Venus,
    }
}

#[test]
fn inferred_hub_schema_is_hub() {
    let hub = make_hub();
    let schema = hub.plugin_schema();
    assert!(
        schema.is_hub(),
        "plugin_schema().is_hub() must be true when #[child] methods are present"
    );
}

#[test]
fn inferred_hub_schema_children_match_static_methods() {
    let hub = make_hub();
    let schema = hub.plugin_schema();
    let children = schema
        .children
        .as_ref()
        .expect("hub schema must carry children");
    assert_eq!(children.len(), 2, "two static #[child] methods → two summaries");

    let names: Vec<&str> = children.iter().map(|c| c.namespace.as_str()).collect();
    assert!(names.contains(&"mercury"));
    assert!(names.contains(&"venus"));

    // Descriptions are sourced from the `#[child]` method's own doc comment
    // (CHILD-5). Our fixture's static children are documented with single-line
    // `///` comments.
    let mercury = children
        .iter()
        .find(|c| c.namespace == "mercury")
        .unwrap();
    assert_eq!(mercury.description, "The Mercury child.");
}

#[tokio::test]
async fn nested_routing_reaches_static_child() {
    let hub = make_hub();
    let result =
        Activation::call(&hub, "mercury.info", serde_json::json!({}), None, None).await;
    assert!(
        result.is_ok(),
        "nested routing must dispatch to the static child (got: {:?})",
        result.err()
    );
}

#[tokio::test]
async fn get_child_still_works_for_static_child() {
    let hub = make_hub();
    // CHILD-3 regression: the generated `ChildRouter::get_child` still
    // answers `Some(_)` for a known static child name.
    assert!(hub.get_child("mercury").await.is_some());
    assert!(hub.get_child("venus").await.is_some());
    assert!(hub.get_child("unknown").await.is_none());
}

// ---------------------------------------------------------------------------
// CHILD-4 regression: an inferred hub with `list = "..."` opt-in produces a
// `Some(stream)` from `list_children`. We add a dynamic `#[child(list = ...)]`
// method alongside static ones and assert the combined stream is produced.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct InferredHubWithListing;

#[plexus_macros::activation(
    namespace = "inferred_hub_listing",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl InferredHubWithListing {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Static child `earth`.
    #[plexus_macros::child]
    fn earth(&self) -> Mercury {
        Mercury
    }

    /// Dynamic child with listing opt-in.
    #[plexus_macros::child(list = "dynamic_names")]
    fn planet(&self, _name: &str) -> Option<Mercury> {
        Some(Mercury)
    }

    fn dynamic_names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! {
            yield "jupiter".to_string();
        }
    }
}

#[tokio::test]
async fn list_children_opt_in_still_yields_some_stream() {
    let hub = InferredHubWithListing;
    let listed: Vec<String> = hub
        .list_children()
        .await
        .expect("list_children must return Some when `list = ...` is set")
        .collect()
        .await;
    // Static names appear first, dynamic names appended (matches CHILD-4
    // semantics tested in child_capabilities_tests.rs).
    assert_eq!(listed, vec!["earth".to_string(), "jupiter".to_string()]);
}

// ---------------------------------------------------------------------------
// Regression: the Solar-pre-migration pattern (explicit `hub` flag +
// hand-written `ChildRouter` impl + hand-written `plugin_children`, NO
// `#[child]` methods) still compiles and behaves as before. Specifically:
//   - The macro does NOT emit its own `ChildRouter` impl (that would collide
//     with the hand-written one).
//   - The macro does NOT synthesize `plugin_children` (user's wins).
//   - `plugin_schema().is_hub()` is `true`, populated from the hand-written
//     `plugin_children`.
//   - `Activation::call(..., "child.method", ...)` routes via the hand-written
//     `ChildRouter`.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct LegacyHub;

#[plexus_macros::activation(
    namespace = "legacy_hub",
    version = "1.0.0",
    description = "Pre-migration hub pattern",
    hub,
    crate_path = "plexus_core"
)]
impl LegacyHub {
    /// Health check.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Hand-written child summaries. The macro must NOT shadow this.
    pub fn plugin_children(&self) -> Vec<plexus_core::plexus::ChildSummary> {
        vec![plexus_core::plexus::ChildSummary {
            namespace: "mercury".to_string(),
            description: "Legacy hand-written summary".to_string(),
            hash: "legacy-hash".to_string(),
        }]
    }
}

#[async_trait::async_trait]
impl plexus_core::plexus::ChildRouter for LegacyHub {
    fn router_namespace(&self) -> &str {
        "legacy_hub"
    }

    async fn router_call(
        &self,
        method: &str,
        params: serde_json::Value,
        auth: Option<&plexus_core::plexus::AuthContext>,
        raw_ctx: Option<&plexus_core::request::RawRequestContext>,
    ) -> Result<plexus_core::plexus::PlexusStream, plexus_core::plexus::PlexusError> {
        <Self as Activation>::call(self, method, params, auth, raw_ctx).await
    }

    async fn get_child(
        &self,
        name: &str,
    ) -> Option<Box<dyn plexus_core::plexus::ChildRouter>> {
        if name == "mercury" {
            // PLX-91: a hand-written router hands the child over OWNED, so the
            // child's turn-native methods can be dispatched through
            // `Activation::call_arc`. `Box::new(Mercury)` used to be enough;
            // since the switchover a macro activation supplies
            // `into_child_router()` for exactly this handoff.
            Some(Mercury.into_child_router())
        } else {
            None
        }
    }
}

#[test]
fn legacy_pattern_schema_is_hub_with_handwritten_children() {
    let hub = LegacyHub;
    let schema = hub.plugin_schema();
    assert!(schema.is_hub());
    let children = schema.children.as_ref().expect("hand-written summaries");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].namespace, "mercury");
    assert_eq!(children[0].description, "Legacy hand-written summary");
    assert_eq!(children[0].hash, "legacy-hash");
}

#[tokio::test]
async fn legacy_pattern_nested_routing_via_handwritten_router() {
    let hub = LegacyHub;
    let result =
        Activation::call(&hub, "mercury.info", serde_json::json!({}), None, None).await;
    assert!(
        result.is_ok(),
        "legacy hub routes through hand-written ChildRouter (got: {:?})",
        result.err()
    );
}
