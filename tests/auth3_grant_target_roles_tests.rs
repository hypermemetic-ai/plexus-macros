//! AUTH-3: the inherent grant, `target = "param"`, the doc comment as
//! `long_description`, `roles(...)` on the activation, and the `reach:
//! Reach` parameter — what the macro puts in the schema, and how it calls.
//!
//! Compile errors (a `target` naming no param, a role listing an unknown
//! method or a role not declared before it, `admin` declared) are trybuild
//! fixtures in `tests/compile/auth3_*.rs`, driven by
//! `auth3_compile_fail` below.
//!
//! Run with: `cargo test --test auth3_grant_target_roles_tests`.

use async_stream::stream;
use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Activation, AuthContext, PlexusStreamItem, Reach};

#[derive(Clone)]
struct Tickets;

#[plexus_macros::activation(
    namespace = "tickets",
    version = "1.0.0",
    crate_path = "plexus_core",
    roles(
        viewer = ["get", "list"],
        editor = [viewer, "set_status"],
    ),
)]
impl Tickets {
    /// One ticket by id
    #[plexus_macros::method(target = "id", params(id = "Ticket id"))]
    async fn get(&self, id: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield id; }
    }

    /// Tickets, answered from where the caller reaches.
    #[plexus_macros::method]
    async fn list(&self, reach: Reach) -> impl Stream<Item = String> + Send + 'static {
        let held: Vec<String> = reach.held_on().iter().map(|s| s.to_string()).collect();
        stream! { yield held.join(","); }
    }

    /// Change a ticket's status. Moves it through Pending → Ready → Coding →
    /// In Review → Complete; a doc has no lifecycle and is refused.
    #[plexus_macros::method(
        description = "Change a ticket's status",
        target = "id",
        params(id = "Ticket id, e.g. MNG-2", status = "New status"),
    )]
    async fn set_status(
        &self,
        auth: &AuthContext,
        reach: Reach,
        id: String,
        status: String,
    ) -> impl Stream<Item = String> + Send + 'static {
        let who = auth.user_id.clone();
        let everywhere = reach.is_everywhere();
        stream! { yield format!("{who} {id} {status} {everywhere}"); }
    }
}

fn method<'a>(schema: &'a plexus_core::plexus::PluginSchema, name: &str) -> &'a plexus_core::plexus::MethodSchema {
    schema.methods.iter().find(|m| m.name == name).unwrap_or_else(|| panic!("no method {name}"))
}

#[test]
fn every_method_carries_its_grant() {
    let schema = Tickets.plugin_schema();
    for m in &schema.methods {
        assert_eq!(m.grant.as_deref(), Some(format!("tickets.{}", m.name).as_str()), "{}", m.name);
    }
}

#[test]
fn target_and_long_description_reach_the_schema() {
    let schema = Tickets.plugin_schema();
    let set_status = method(&schema, "set_status");
    assert_eq!(set_status.target.as_deref(), Some("id"));
    assert_eq!(set_status.description, "Change a ticket's status");
    assert_eq!(
        set_status.long_description.as_deref(),
        Some("Moves it through Pending → Ready → Coding →\nIn Review → Complete; a doc has no lifecycle and is refused.")
    );
    // A doc comment that IS the description adds nothing.
    let get = method(&schema, "get");
    assert_eq!(get.description, "One ticket by id");
    assert_eq!(get.long_description, None);
    assert_eq!(method(&schema, "list").target, None);
}

#[test]
fn reach_and_auth_are_not_params() {
    let schema = Tickets.plugin_schema();
    let props = |name: &str| -> Vec<String> {
        let Some(params) = method(&schema, name).params.as_ref() else { return Vec::new() };
        let params = serde_json::to_value(params).unwrap();
        let mut keys: Vec<String> = params["properties"].as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
        keys.sort();
        keys
    };
    assert_eq!(props("set_status"), vec!["id", "status"]);
    assert!(props("list").is_empty());
}

#[test]
fn roles_are_flattened_to_grants() {
    let schema = Tickets.plugin_schema();
    assert_eq!(schema.roles.len(), 2);
    assert_eq!(schema.roles["viewer"], vec!["tickets.get", "tickets.list"]);
    assert_eq!(schema.roles["editor"], vec!["tickets.get", "tickets.list", "tickets.set_status"]);
}

/// The wire shape, pinned: what a client reads for `set_status` and the
/// activation's roles.
#[test]
fn schema_snapshot() {
    let schema = Tickets.plugin_schema();
    let json = serde_json::to_value(&schema).unwrap();
    assert_eq!(
        json["roles"],
        serde_json::json!({
            "editor": ["tickets.get", "tickets.list", "tickets.set_status"],
            "viewer": ["tickets.get", "tickets.list"],
        })
    );
    let m = json["methods"].as_array().unwrap().iter().find(|m| m["name"] == "set_status").unwrap().clone();
    let pick = |k: &str| m.get(k).cloned().unwrap_or(serde_json::Value::Null);
    assert_eq!(
        serde_json::json!({
            "name": pick("name"),
            "description": pick("description"),
            "long_description": pick("long_description"),
            "grant": pick("grant"),
            "target": pick("target"),
            "required": m["params"]["required"].clone(),
        }),
        serde_json::json!({
            "name": "set_status",
            "description": "Change a ticket's status",
            "long_description": "Moves it through Pending → Ready → Coding →\nIn Review → Complete; a doc has no lifecycle and is refused.",
            "grant": "tickets.set_status",
            "target": "id",
            "required": ["id", "status"],
        })
    );
    // A method without a target, a long description or roles says nothing of them.
    let list = json["methods"].as_array().unwrap().iter().find(|m| m["name"] == "list").unwrap();
    assert!(list.get("target").is_none() && list.get("long_description").is_none());
    assert_eq!(list["grant"], "tickets.list");
}

async fn first_data(stream: plexus_core::plexus::PlexusStream) -> serde_json::Value {
    let mut s = stream;
    while let Some(item) = s.next().await {
        if let PlexusStreamItem::Data { content, .. } = item {
            return content;
        }
    }
    panic!("no data item");
}

/// Outside a gated dispatch, a method that takes `reach` gets everywhere —
/// what an activation always meant without a gate.
#[tokio::test]
async fn without_a_gate_reach_is_everywhere() {
    let out = first_data(Tickets.call("list", serde_json::json!({}), None, None).await.unwrap()).await;
    assert_eq!(out, "*");
    let auth = AuthContext::new("u1".into(), "s1".into(), vec![], serde_json::Value::Null);
    let out = first_data(
        Tickets.call("set_status", serde_json::json!({"id": "MNG-2", "status": "Ready"}), Some(&auth), None).await.unwrap(),
    )
    .await;
    assert_eq!(out, "u1 MNG-2 Ready true");
}

#[test]
fn auth3_compile_fail() {
    let t = trybuild::TestCases::new();
    t.compile_fail("tests/compile/auth3_target_unknown_param.rs");
    t.compile_fail("tests/compile/auth3_role_unknown_method.rs");
    t.compile_fail("tests/compile/auth3_role_includes_later_role.rs");
    t.compile_fail("tests/compile/auth3_role_admin_declared.rs");
    t.compile_fail("tests/compile/auth3_reach_out_of_place.rs");
}
