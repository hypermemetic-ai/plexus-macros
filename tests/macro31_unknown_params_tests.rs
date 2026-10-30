//! MACRO-31 · under `typed_params` a key no parameter claims is refused.
//!
//! MACRO-29 measured the hole: the dispatch reads each parameter by name and
//! never looks at the other keys, so `tickets.list {sumary: true}` answered
//! as if no `summary` were given. Under `typed_params` an object's key that
//! names no wire parameter of the method is `InvalidParams` (-32602) before
//! the method runs: ``unknown field `sumary`; tickets.list takes: after,
//! limit, summary`` — every unknown key named, the method's parameters
//! sorted. `unknown field` is the words synapse reads to know its cached
//! schema is stale (it re-reads it and runs the call again), so they are
//! there verbatim. `null`, `{}` and the bare-value shortcut are answered as
//! before (`null` to a method of many parameters stays `expected object`); what the server injects (`#[from_auth]`, `auth`, `reach`, a
//! bidirectional `ctx`) is no wire parameter and no key. An activation
//! without `typed_params` is unchanged.

use futures::stream::Stream;
use plexus_core::plexus::{Activation, PlexusError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A cursor: opaque on the wire.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct Cursor(u32);

/// A ticket id.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct TicketId(u32);

/// A small tickets service under `typed_params`.
#[derive(Clone)]
pub struct TicketsActivation;

#[plexus_macros::activation(crate_path = "plexus_core", typed_params)]
impl TicketsActivation {
    /// No parameters.
    pub async fn health(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "ok".to_string(); }
    }

    /// One parameter.
    pub async fn get(&self, id: TicketId) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{}", id.0); }
    }

    /// One optional parameter.
    pub async fn maybe(&self, keep: Option<TicketId>) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{:?}", keep.map(|k| k.0)); }
    }

    /// Many parameters, declared out of order.
    pub async fn list(
        &self,
        summary: Option<bool>,
        limit: Option<u32>,
        after: Option<Cursor>,
    ) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{summary:?} {limit:?} {:?}", after.map(|a| a.0)); }
    }
}

/// The same methods with no `typed_params`: nothing changes.
#[derive(Clone)]
pub struct LooseActivation;

#[plexus_macros::activation(crate_path = "plexus_core")]
impl LooseActivation {
    /// No parameters.
    pub async fn health(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "ok".to_string(); }
    }

    /// One parameter.
    pub async fn get(&self, id: TicketId) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{}", id.0); }
    }

    /// Many parameters.
    pub async fn list(
        &self,
        summary: Option<bool>,
        limit: Option<u32>,
        after: Option<Cursor>,
    ) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield format!("{summary:?} {limit:?} {:?}", after.map(|a| a.0)); }
    }
}

async fn refused<A: Activation>(a: &A, method: &str, params: serde_json::Value) -> String {
    match a.call(method, params.clone(), None, None).await {
        Err(PlexusError::InvalidParams(m)) => m,
        Err(other) => panic!("{method} {params}: expected InvalidParams, got {other:?}"),
        Ok(_) => panic!("{method} {params}: expected InvalidParams, the method ran"),
    }
}

async fn ran<A: Activation>(a: &A, method: &str, params: serde_json::Value) {
    if let Err(e) = a.call(method, params.clone(), None, None).await {
        panic!("{method} {params}: expected the method to run, got {e:?}");
    }
}

#[tokio::test]
async fn one_parameter_refuses_an_unknown_key() {
    let m = refused(&TicketsActivation, "get", serde_json::json!({ "id": 7, "idd": 8 })).await;
    assert_eq!(m, "unknown field `idd`; tickets.get takes: id");
    let m = refused(&TicketsActivation, "maybe", serde_json::json!({ "kep": 7 })).await;
    assert_eq!(m, "unknown field `kep`; tickets.maybe takes: keep");
    // A required parameter missing and an unknown key: the unknown key is said
    // first, so a stale cache heals instead of reporting the missing one.
    let m = refused(&TicketsActivation, "get", serde_json::json!({ "ident": 7 })).await;
    assert_eq!(m, "unknown field `ident`; tickets.get takes: id");
}

#[tokio::test]
async fn many_parameters_refuse_an_unknown_key_naming_them_all() {
    let m = refused(&TicketsActivation, "list", serde_json::json!({ "sumary": true })).await;
    assert_eq!(m, "unknown field `sumary`; tickets.list takes: after, limit, summary");
    let m = refused(&TicketsActivation, "list", serde_json::json!({ "summary": true, "limt": 1 })).await;
    assert_eq!(m, "unknown field `limt`; tickets.list takes: after, limit, summary");
    // Every unknown key is named, sorted.
    let m = refused(&TicketsActivation, "list", serde_json::json!({ "sumary": true, "limt": 1, "after": 3 })).await;
    assert_eq!(m, "unknown field `limt`, `sumary`; tickets.list takes: after, limit, summary");
}

#[tokio::test]
async fn zero_parameters_refuse_any_key() {
    let m = refused(&TicketsActivation, "health", serde_json::json!({ "foo": 1 })).await;
    assert_eq!(m, "unknown field `foo`; tickets.health takes no parameters");
    // An explicit null under a key is still a key.
    let m = refused(&TicketsActivation, "health", serde_json::json!({ "foo": null })).await;
    assert_eq!(m, "unknown field `foo`; tickets.health takes no parameters");
}

#[tokio::test]
async fn null_and_empty_are_accepted() {
    for p in [serde_json::Value::Null, serde_json::json!({})] {
        ran(&TicketsActivation, "health", p.clone()).await;
        ran(&TicketsActivation, "maybe", p.clone()).await;
    }
    ran(&TicketsActivation, "list", serde_json::json!({})).await;
    // A method of many parameters refused `null` before MACRO-31 and still
    // does, in the same words: the check adds no acceptance either.
    let m = refused(&TicketsActivation, "list", serde_json::Value::Null).await;
    assert_eq!(m, "expected object");
    // Every known key, an explicit null among them.
    ran(&TicketsActivation, "list", serde_json::json!({ "summary": true, "limit": 1, "after": null })).await;
    ran(&TicketsActivation, "get", serde_json::json!({ "id": 7 })).await;
}

#[tokio::test]
async fn the_bare_value_shortcut_is_accepted() {
    ran(&TicketsActivation, "get", serde_json::json!(7)).await;
    ran(&TicketsActivation, "maybe", serde_json::json!(7)).await;
}

#[tokio::test]
async fn without_typed_params_nothing_changes() {
    ran(&LooseActivation, "health", serde_json::json!({ "foo": 1 })).await;
    ran(&LooseActivation, "get", serde_json::json!({ "id": 7, "idd": 8 })).await;
    ran(&LooseActivation, "list", serde_json::json!({ "sumary": true, "limt": 1 })).await;
    // A missing required parameter still says so.
    let m = refused(&LooseActivation, "get", serde_json::json!({ "ident": 7 })).await;
    assert_eq!(m, "missing field: id");
}

#[tokio::test]
async fn the_message_carries_unknown_field() {
    let e = PlexusError::InvalidParams(refused(&TicketsActivation, "list", serde_json::json!({ "sumary": true })).await);
    let item = plexus_core::plexus::error_item("tickets", &e);
    assert_eq!(item["code"], "-32602");
    assert_eq!(item["error_kind"], "invalid_params");
    let message = item["message"].as_str().expect("a message");
    assert_eq!(message, "Invalid params: unknown field `sumary`; tickets.list takes: after, limit, summary");
    // synapse's heal word, verbatim (synapse/app/Main.hs, isStaleItem).
    assert!(message.contains("unknown field"), "{message}");
}

#[test]
fn the_readme_says_an_unknown_key_is_refused() {
    let readme = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/README.md")).expect("README.md");
    let row = readme
        .lines()
        .find(|l| l.starts_with("| `typed_params` |"))
        .expect("README's attribute table has a `typed_params` row");
    assert!(row.contains("MACRO-31"), "the `typed_params` row names MACRO-31: {row}");
    assert!(row.contains("unknown field"), "the `typed_params` row gives the refusal's words: {row}");
}
