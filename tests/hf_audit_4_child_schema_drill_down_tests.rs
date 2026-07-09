//! PROT schema unification (PLX-13) — schema drill-down / `<name>.schema` tests.
//!
//! History: HF-AUDIT-3 diagnosed that the generated `Activation::call` `_` arm
//! carried a `strip_suffix(".schema")` shortcut that returned a bare
//! `SchemaResult::Method` for `<name>.schema`. Since CHILD-8 / IR-10 emitted
//! every `#[child]` accessor as a `MethodSchema` entry, the shortcut's
//! `find(|m| m.name == method_name)` matched child accessors and short-circuited
//! dispatch, so `synapse lforge hyperforge build` saw a `*.method_schema`
//! content_type ("no schema in response"). HF-AUDIT-4 (0.5.2) narrowed the
//! predicate with a child-role guard as a stopgap.
//!
//! PLX-13 deletes the shortcut and its role-guard entirely, together with the
//! `SchemaResult::Method` variant and the `.method_schema` content-type. The
//! unified contract:
//!   * `<child>.schema` still drills into the child and returns the child's own
//!     `SchemaResult::Plugin(<child_plugin_schema>)` — now via `route_to_child`
//!     directly (the shortcut that had to be guarded against is simply gone).
//!   * `<method>.schema` for a plain local `#[method]` is no longer a per-method
//!     result path: it is method-not-found. Per-method detail is read from the
//!     `methods[]` of the hub's own unified `.schema`.

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Activation, PlexusError, PlexusStreamItem, SchemaResult};

// ---------------------------------------------------------------------------
// Leaf child with its own local methods, so the test can assert that
// drilling in returns the CHILD's plugin_schema.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(
    namespace = "hf_audit_4_leaf",
    version = "1.0.0",
    description = "Leaf activation used as the child target for the drill-down regression.",
    crate_path = "plexus_core"
)]
impl Leaf {
    /// Leaf's own user method. Must appear in the leaf's plugin_schema
    /// when a caller drills into it via `<parent>.<leaf>.schema`.
    #[plexus_macros::method]
    async fn info(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "leaf-info".to_string(); }
    }
}

// ---------------------------------------------------------------------------
// Parent hub with one static `#[child]` accessor AND one regular
// `#[method]`.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Hub {
    leaf: Leaf,
}

#[plexus_macros::activation(
    namespace = "hf_audit_4_hub",
    version = "1.0.0",
    description = "Hub with one static child and one regular method.",
    crate_path = "plexus_core"
)]
impl Hub {
    /// Regular user method. After PLX-13, `"echo.schema"` is method-not-found
    /// (no per-method shortcut); per-method detail lives in the hub's own
    /// unified `.schema` `methods[]`.
    #[plexus_macros::method]
    async fn echo(&self, msg: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield msg; }
    }

    /// Static child accessor — `"leaf.schema"` must drill into Leaf and
    /// return `SchemaResult::Plugin(<leaf_plugin_schema>)`.
    #[plexus_macros::child]
    fn leaf(&self) -> Leaf {
        self.leaf.clone()
    }
}

// ---------------------------------------------------------------------------
// Helper: drain a PlexusStream and extract the first `Data` payload,
// deserialized as `SchemaResult`.
// ---------------------------------------------------------------------------

async fn call_for_schema(hub: &Hub, method: &str) -> SchemaResult {
    let stream = Activation::call(hub, method, serde_json::json!({}), None, None)
        .await
        .unwrap_or_else(|e| panic!("call({method}) failed: {e:?}"));

    let items: Vec<PlexusStreamItem> = stream.collect().await;
    for item in items {
        if let PlexusStreamItem::Data { content, .. } = item {
            return serde_json::from_value(content)
                .unwrap_or_else(|e| panic!("could not deserialize SchemaResult: {e:?}"));
        }
    }
    panic!("no Data item in stream for {method}");
}

// ---------------------------------------------------------------------------
// <child>.schema still drills into the child's PluginSchema (now the default
// path — there is no shortcut to guard against).
// ---------------------------------------------------------------------------

#[tokio::test]
async fn child_schema_drills_into_child_plugin_schema() {
    let hub = Hub { leaf: Leaf };
    let result = call_for_schema(&hub, "leaf.schema").await;

    // PLX-13: SchemaResult is Plugin-only; the match is exhaustive.
    let SchemaResult::Plugin(plugin) = result;

    assert_eq!(
        plugin.namespace, "hf_audit_4_leaf",
        "drill-in must return the LEAF's plugin_schema, not the hub's"
    );
    assert!(
        plugin.methods.iter().any(|m| m.name == "info"),
        "leaf's plugin_schema must list its own `info` method (got: {:?})",
        plugin.methods.iter().map(|m| &m.name).collect::<Vec<_>>()
    );
}

// ---------------------------------------------------------------------------
// PLX-13: a plain `<method>.schema` is no longer a per-method result path.
// The strip-suffix shortcut and the `.method_schema` content-type are gone,
// so `echo.schema` (echo is a leaf `#[method]`, not a child) is method-not-found.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn regular_method_dot_schema_is_now_method_not_found() {
    let hub = Hub { leaf: Leaf };
    let outcome = Activation::call(&hub, "echo.schema", serde_json::json!({}), None, None).await;

    // With the shortcut deleted, `echo.schema` is no longer a per-method result
    // path. On a hub it falls through to `route_to_child`, which finds no child
    // named `echo` and rejects the call as not-found (`ActivationNotFound` here;
    // a leaf activation would reject it as `MethodNotFound`). Either way it must
    // be an error that names `echo`, never a schema payload.
    match outcome {
        Err(err @ (PlexusError::MethodNotFound { .. } | PlexusError::ActivationNotFound(_))) => {
            assert!(
                format!("{err:?}").contains("echo"),
                "the not-found error should name the unresolved `echo.schema` call, got: {err:?}"
            );
        }
        Err(other) => panic!(
            "expected a not-found rejection for the deleted `<method>.schema` shortcut, got a different error: {other:?}"
        ),
        Ok(_) => panic!(
            "PLX-13 regression: `echo.schema` still resolved — the strip-suffix \
             per-method shortcut must be deleted (per-method detail is read from \
             the hub's unified `.schema` `methods[]`)."
        ),
    }
}
