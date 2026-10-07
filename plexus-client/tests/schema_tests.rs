//! RETURNS-4 acceptance 4: under the `schema` feature, `Note` and
//! `RemoteError` implement `schemars::JsonSchema`, and the schema describes
//! the wire shape serde produces.
//!
//! Run with `cargo test -p plexus-client --features schema`; without the
//! feature this file compiles to nothing.

#![cfg(feature = "schema")]

use plexus_client::{DenyReason, Note, ParamProblem, RemoteError};
use schemars::{schema_for, JsonSchema};
use serde_json::Value;

fn implements_json_schema<T: JsonSchema>() {}

#[test]
fn the_vocabulary_implements_json_schema() {
    implements_json_schema::<Note>();
    implements_json_schema::<RemoteError>();
    implements_json_schema::<ParamProblem>();
    implements_json_schema::<DenyReason>();
}

#[test]
fn note_schema_requires_only_message() {
    let schema = serde_json::to_value(schema_for!(Note)).unwrap();
    assert_eq!(
        schema["required"],
        serde_json::json!(["message"]),
        "{schema:#}"
    );
    assert!(schema["properties"]["percentage"].is_object(), "{schema:#}");
}

/// Every `kind` the serde encoding emits appears as a constant in the
/// schema — the schema and the wire name the same variants.
#[test]
fn remote_error_schema_names_every_kind() {
    let text = serde_json::to_string(&schema_for!(RemoteError)).unwrap();
    for kind in [
        "activation_not_found",
        "method_not_found",
        "invalid_params",
        "unauthenticated",
        "forbidden",
        "handle_not_supported",
        "execution",
        // flattened tags
        "missing",
        "malformed",
        "missing_scope",
        "tenant_boundary",
        "not_accepted",
    ] {
        assert!(
            text.contains(&format!("\"{kind}\"")),
            "{kind} missing from {text}"
        );
    }
    let schema: Value = serde_json::from_str(&text).unwrap();
    assert!(schema.is_object());
}
