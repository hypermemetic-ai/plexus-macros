//! SYNAPSE-13: `group = "…"`, `example(note = "…", params = r#"{…}"#)`
//! (repeatable) and `values(param = ["…", …])` on `#[plexus_macros::method]`
//! — what the macro puts in the schema (`group`, `examples`, and
//! `x-plexus-values` on a param's property), what it leaves alone (a method
//! without them keeps its wire shape), and that the values are a hint the
//! dispatch never enforces.
//!
//! Compile errors (malformed or misnamed attributes) are trybuild fixtures
//! in `tests/compile/synapse13_*.rs`, driven by `synapse13_compile_fail`.
//!
//! Run with: `cargo test --test synapse13_group_examples_values_tests`.

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Activation, MethodExample, PlexusStreamItem, X_PLEXUS_VALUES};

#[derive(Clone)]
struct Tickets;

#[plexus_macros::activation(namespace = "tickets", version = "1.0.0", crate_path = "plexus_core")]
impl Tickets {
    /// One ticket by id
    #[plexus_macros::method(
        group = "read",
        params(id = "Ticket id, e.g. MNG-2"),
        example(note = "one ticket", params = r#"{"id": "MENGER-40"}"#),
    )]
    async fn get(&self, id: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield id; }
    }

    /// Change a ticket's status
    #[plexus_macros::method(
        group = "write",
        target = "id",
        params(id = "Ticket id", status = "New status"),
        values(status = ["Pending", "Ready", "Coding"]),
        example(note = "start work on MENGER-40", params = r#"{"id": "MENGER-40", "status": "Coding"}"#),
        example(params = r#"{"id": "MENGER-41", "status": "Ready"}"#),
    )]
    async fn set_status(&self, id: String, status: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield format!("{id} {status}"); }
    }

    /// Nothing new declared
    #[plexus_macros::method(params(kind = "A kind"))]
    async fn plain(&self, kind: Option<String>) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield kind.unwrap_or_default(); }
    }
}

fn method(name: &str) -> plexus_core::plexus::MethodSchema {
    Tickets.plugin_schema().methods.into_iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no method {name}"))
}

#[test]
fn group_reaches_the_schema() {
    assert_eq!(method("get").group.as_deref(), Some("read"));
    assert_eq!(method("set_status").group.as_deref(), Some("write"));
    assert_eq!(method("plain").group, None);
    // The auto-injected schema method declares none.
    assert_eq!(method("schema").group, None);
}

#[test]
fn examples_reach_the_schema_in_order() {
    let set_status = method("set_status");
    assert_eq!(
        set_status.examples,
        vec![
            MethodExample::with_note(serde_json::json!({"id": "MENGER-40", "status": "Coding"}), "start work on MENGER-40"),
            MethodExample::new(serde_json::json!({"id": "MENGER-41", "status": "Ready"})),
        ]
    );
    assert_eq!(method("get").examples.len(), 1);
    assert!(method("plain").examples.is_empty());
}

#[test]
fn values_are_x_plexus_values_on_the_property() {
    let set_status = method("set_status");
    assert_eq!(set_status.param_values("status"), Some(vec!["Pending".into(), "Ready".into(), "Coding".into()]));
    assert_eq!(set_status.param_values("id"), None);
    let params = serde_json::to_value(set_status.params.as_ref().unwrap()).unwrap();
    // Beside what schemars wrote (the type and description stay), and not
    // an enum: a hint, never a constraint.
    let status = &params["properties"]["status"];
    assert_eq!(status["type"], "string");
    assert_eq!(status["description"], "New status");
    assert_eq!(status[X_PLEXUS_VALUES], serde_json::json!(["Pending", "Ready", "Coding"]));
    assert!(status.get("enum").is_none());
}

/// The wire shape, pinned: `set_status` with all three, `plain` with none
/// (no `group`, no `examples`, no `x-plexus-values` key at all).
#[test]
fn schema_snapshot() {
    let json = serde_json::to_value(Tickets.plugin_schema()).unwrap();
    let find = |name: &str| json["methods"].as_array().unwrap().iter().find(|m| m["name"] == name).unwrap().clone();
    let m = find("set_status");
    assert_eq!(
        serde_json::json!({
            "group": m["group"],
            "examples": m["examples"],
            "status": m["params"]["properties"]["status"],
        }),
        serde_json::json!({
            "group": "write",
            "examples": [
                { "params": { "id": "MENGER-40", "status": "Coding" }, "note": "start work on MENGER-40" },
                { "params": { "id": "MENGER-41", "status": "Ready" } },
            ],
            "status": { "type": "string", "description": "New status", "x-plexus-values": ["Pending", "Ready", "Coding"] },
        })
    );
    let plain = find("plain");
    assert!(plain.get("group").is_none() && plain.get("examples").is_none());
    assert!(!plain["params"].to_string().contains(X_PLEXUS_VALUES));
}

/// A schema without the fields (an older server's) parses, and says none.
#[test]
fn an_older_schema_parses() {
    let old = serde_json::json!({ "name": "get", "description": "One", "hash": "00" });
    let m: plexus_core::plexus::MethodSchema = serde_json::from_value(old).unwrap();
    assert_eq!(m.group, None);
    assert!(m.examples.is_empty());
}

/// The values are a hint: a value outside them still reaches the method.
#[tokio::test]
async fn values_are_not_enforced() {
    let mut s = Tickets.call("set_status", serde_json::json!({"id": "MNG-2", "status": "active"}), None, None).await.unwrap();
    while let Some(item) = s.next().await {
        if let PlexusStreamItem::Data { content, .. } = item {
            assert_eq!(content, "MNG-2 active");
            return;
        }
    }
    panic!("no data item");
}

/// Declaring them changes the method's hash (a client's cached schema is
/// refetched); a method without them hashes as it did.
#[test]
fn they_are_part_of_the_hash() {
    #[derive(Clone)]
    struct Bare;
    #[plexus_macros::activation(namespace = "tickets", version = "1.0.0", crate_path = "plexus_core")]
    impl Bare {
        /// Change a ticket's status
        #[plexus_macros::method(target = "id", params(id = "Ticket id", status = "New status"))]
        async fn set_status(&self, id: String, status: String) -> impl Stream<Item = String> + Send + 'static {
            stream! { yield format!("{id} {status}"); }
        }
        /// Nothing new declared
        #[plexus_macros::method(params(kind = "A kind"))]
        async fn plain(&self, kind: Option<String>) -> impl Stream<Item = String> + Send + 'static {
            stream! { yield kind.unwrap_or_default(); }
        }
    }
    let bare = |n: &str| Bare.plugin_schema().methods.into_iter().find(|m| m.name == n).unwrap().hash;
    assert_ne!(bare("set_status"), method("set_status").hash);
    assert_eq!(bare("plain"), method("plain").hash);
}

#[test]
fn synapse13_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile/synapse13_values_unknown_param.rs");
    t.compile_fail("tests/compile/synapse13_values_not_strings.rs");
    t.compile_fail("tests/compile/synapse13_example_not_json.rs");
    t.compile_fail("tests/compile/synapse13_example_not_object.rs");
    t.compile_fail("tests/compile/synapse13_example_unknown_param.rs");
    t.compile_fail("tests/compile/synapse13_example_no_params.rs");
    t.compile_fail("tests/compile/synapse13_example_unknown_key.rs");
    t.compile_fail("tests/compile/synapse13_group_not_a_string.rs");
}
