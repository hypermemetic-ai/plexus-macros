//! RETURNS-8 acceptance 3: the implicit namespace (RETURNS-1 decision 7) —
//! and `namespace = "..."` used as given. One activation per case; each is
//! checked where the namespace shows: the `NAMESPACE` const,
//! `Activation::namespace()`, the schema, and a call's `content_type`.
//!
//! RETURNS-36: main's MACRO-13 name rule wins the merge (naming is main's):
//! the namespace is the stem of `impl FooActivation`, and a type the rule
//! cannot name, without `namespace`, is refused (MACRO-13's
//! `macro13_cannot_name_the_trait` compile test). So `Tickets` → `tickets`
//! and `HyperforgeHub` → `hyperforge` are gone — the first is refused, the
//! second ends in `Hub`, a word NAMING-10 retired.

use futures::StreamExt;
use plexus_core::plexus::{Activation, PlexusStreamItem};

/// `InfoActivation` → `info`.
#[derive(Clone)]
pub struct InfoActivation;

#[plexus_macros::activation]
impl InfoActivation {
    /// Health.
    #[plexus_macros::method]
    async fn health(&self) -> String {
        "ok".into()
    }
}

/// `ClaudeCode` with `namespace = "claudecode"` → `claudecode` (implicitly
/// it would be `claude_code`).
pub struct ClaudeCode;

#[plexus_macros::activation(namespace = "claudecode")]
impl ClaudeCode {
    /// Health.
    #[plexus_macros::method]
    async fn health(&self) -> String {
        "ok".into()
    }
}

async fn assert_namespace<A: Activation>(activation: A, constant: &str, expected: &str) {
    assert_eq!(constant, expected, "NAMESPACE");
    assert_eq!(activation.namespace(), expected, "Activation::namespace()");
    assert_eq!(activation.plugin_schema().namespace, expected, "plugin_schema().namespace");

    let frames: Vec<PlexusStreamItem> = activation
        .call("health", serde_json::json!({}), None, None)
        .await
        .expect("health dispatches")
        .collect()
        .await;
    match &frames[0] {
        PlexusStreamItem::Data { content_type, metadata, .. } => {
            assert_eq!(content_type, &format!("{expected}.health"), "content_type");
            assert_eq!(metadata.provenance, vec![expected.to_string()], "provenance");
        }
        other => panic!("expected a data frame, got {other:?}"),
    }
}

#[tokio::test]
async fn info_activation_is_info() {
    assert_namespace(InfoActivation, InfoActivation::NAMESPACE, "info").await;
}

#[tokio::test]
async fn claude_code_with_an_explicit_namespace_is_claudecode() {
    assert_namespace(ClaudeCode, ClaudeCode::NAMESPACE, "claudecode").await;
}
