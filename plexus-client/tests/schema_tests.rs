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

/// RETURNS-36 (RETURNS-40): the schema is one synapse's IR reader takes —
/// `RemoteError` an untagged union of two references, the known kinds a
/// tagged union whose every member gives `kind` a const, and `OtherError` a
/// struct of `kind` with no map beside it.
#[test]
fn remote_error_schema_is_one_synapse_reads() {
    let schema = serde_json::to_value(schema_for!(RemoteError)).unwrap();
    let defs = &schema["$defs"];
    let members = schema["anyOf"].as_array().expect("an anyOf");
    let refs: Vec<&str> = members.iter().map(|m| m["$ref"].as_str().expect("each member a $ref")).collect();
    assert_eq!(refs, ["#/$defs/KnownRemoteError", "#/$defs/OtherError"]);
    let known = defs["KnownRemoteError"]["oneOf"].as_array().expect("the known kinds a oneOf");
    assert_eq!(known.len(), 10);
    for v in known {
        assert!(v["properties"]["kind"]["const"].is_string(), "every known kind a const: {v}");
    }
    let other = &defs["OtherError"];
    assert_eq!(other["properties"]["kind"]["type"], "string");
    assert!(other.get("additionalProperties").is_none(), "no map beside properties: {other}");
}
