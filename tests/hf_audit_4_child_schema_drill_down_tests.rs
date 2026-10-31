//! HF-AUDIT-4 regression tests.
//!
//! The `.schema` strip-suffix shortcut in the generated `Activation::call`
//! `_` arm (`plexus-macros/src/codegen/activation.rs`) must skip methods
//! whose `role` is `StaticChild` / `DynamicChild { .. }`. Otherwise a call
//! like `<parent>.<child>.schema` returns `SchemaResult::Method` for the
//! child accessor instead of drilling into the child and returning the
//! child's own `PluginSchema`.
//!
//! Pre-HF-AUDIT-4 bug (shipped in plexus-macros 0.5.1, diagnosed in
//! HF-AUDIT-3): since CHILD-8 / IR-10, every static and dynamic `#[child]`
//! accessor is emitted as a `MethodSchema` entry. The shortcut's
//! unconditional `find(|m| m.name == method_name)` matched those entries
//! and short-circuited the dispatch, so `synapse lforge hyperforge build`
//! saw a content_type of `*.method_schema` (no schema in response) instead
//! of BuildHub's `*.schema` tree.
//!
//! Fix: narrow the `find` predicate to exclude child-role entries. Regular
//! `#[method]`s still hit the shortcut; `#[child]` accessors fall through
//! to `route_to_child`, which dispatches `"<child>.schema"` into the child
//! activation and gets back `SchemaResult::Plugin(<child_plugin_schema>)`.

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Activation, PlexusStreamItem, SchemaResult};

// ---------------------------------------------------------------------------
// Leaf child with its own local methods, so the test can assert that
// drilling in returns the CHILD's plugin_schema (not the parent's
// child-accessor method_schema).
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(
    namespace = "hf_audit_4_leaf",
    version = "1.0.0",
    description = "Leaf activation used as the child target for HF-AUDIT-4 regression.",
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
// `#[method]`. Both names survive strip_suffix(".schema"); only the
// non-child one must hit the shortcut.
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
    /// Regular user method — `"echo.schema"` must still return a
    /// `SchemaResult::Method` (the shortcut path, unchanged).
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
            return serde_json::from_value(content.into_value())
                .unwrap_or_else(|e| panic!("could not deserialize SchemaResult: {e:?}"));
        }
    }
    panic!("no Data item in stream for {method}");
}

// ---------------------------------------------------------------------------
// AC #3: <child>.schema must drill into the child's PluginSchema, NOT
// return a MethodSchema for the child accessor.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn child_schema_drills_into_child_plugin_schema() {
    let hub = Hub { leaf: Leaf };
    let result = call_for_schema(&hub, "leaf.schema").await;

    let plugin = match result {
        SchemaResult::Plugin(p) => p,
        SchemaResult::Method(m) => panic!(
            "HF-AUDIT-4 regression: `leaf.schema` returned MethodSchema \
             (name={}, role={:?}) instead of drilling into the child's \
             PluginSchema. The `.schema` strip-suffix shortcut is \
             matching child-role methods again.",
            m.name, m.role,
        ),
    };

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
// AC #3: regular `<method>.schema` must STILL return the method's schema
// via the shortcut. This pins the unchanged path so the narrow fix doesn't
// regress the feature the shortcut was added for.
// ---------------------------------------------------------------------------

#[tokio::test]
async fn regular_method_schema_still_returns_method_schema() {
    let hub = Hub { leaf: Leaf };
    let result = call_for_schema(&hub, "echo.schema").await;

    let method = match result {
        SchemaResult::Method(m) => m,
        SchemaResult::Plugin(p) => panic!(
            "`echo.schema` returned a PluginSchema (namespace={}) instead \
             of the per-method MethodSchema. The shortcut is no longer \
             firing for regular `#[method]`s.",
            p.namespace,
        ),
    };

    assert_eq!(method.name, "echo");
}
