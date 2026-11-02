//! SYNAPSE-CC-9 · the auto `schema` method declares its param and its return
//! (`{method?: string}` → `SchemaResult`, as RETURNS-26 did on 0.7), and a
//! `serde_json::Value` parameter is the JSON type — `{"type": JSON_TYPES}`,
//! its doc beside it; no mark, no compile error (the owner, 2026-10-29:
//! nothing is opaque).

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::schema::{foreign_keywords, spell_json_values, untyped_sites, SchemaParams};
use plexus_core::plexus::{Activation, SchemaResult, JSON_TYPES};
use schemars::schema_for;
use serde_json::{json, Value};

#[derive(Clone)]
struct Store;

#[plexus_macros::activation(namespace = "store", version = "1.0.0", crate_path = "plexus_core")]
impl Store {
    /// Put a value the caller shapes
    #[plexus_macros::method(params(key = "Its key"))]
    async fn put(
        &self,
        key: String,
        /// The value, as the caller shapes it: any JSON
        value: Value,
        /// Labels, if any
        tags: Option<Vec<Value>>,
    ) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield format!("{key} {value} {tags:?}"); }
    }

    /// Nothing dynamic
    #[plexus_macros::method]
    async fn get(&self, key: String) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield key; }
    }
}

fn method(name: &str) -> plexus_core::plexus::MethodSchema {
    Store.plugin_schema().methods.into_iter().find(|m| m.name == name).unwrap()
}

#[test]
fn the_schema_method_declares_its_param_and_return() {
    let s = method("schema");
    let params = serde_json::to_value(s.params.as_ref().expect("params")).unwrap();
    assert_eq!(params["type"], json!("object"));
    assert_eq!(params["properties"]["method"]["type"], json!(["string", "null"]));
    assert!(params.get("required").is_none(), "method is optional: {params}");
    assert_eq!(s.params, Some(SchemaParams::params_schema()));
    let mut result = schema_for!(SchemaResult);
    spell_json_values(&mut result); // its JSON-Schema-valued fields are the JSON type
    assert_eq!(s.returns, result);
    assert_eq!(s.hash, "auto_schema");
    assert_eq!(s.grant.as_deref(), Some("store.schema"));
    assert!(s.input.is_none());
}

#[test]
fn a_value_param_is_the_json_type() {
    let p = serde_json::to_value(method("put").params.unwrap()).unwrap();
    let value = &p["properties"]["value"];
    assert_eq!(value, &json!({"description": "The value, as the caller shapes it: any JSON", "type": JSON_TYPES}), "{p}");
    assert_eq!(JSON_TYPES, ["null", "boolean", "number", "string", "array", "object"]);
    assert!(p["required"].as_array().unwrap().contains(&json!("value")));
    // inside an Option<Vec<…>>: the element is the JSON type
    let tags = &p["properties"]["tags"];
    assert_eq!(tags["items"], json!({"type": JSON_TYPES}), "{tags}");
    // nothing else on the property, and the key untouched
    assert_eq!(p["properties"]["key"], json!({"description": "Its key", "type": "string"}));
}

#[test]
fn the_activation_has_no_untyped_site_and_no_foreign_keyword() {
    let plugin = Store.plugin_schema();
    assert_eq!(untyped_sites(&plugin), Vec::<String>::new());
    assert_eq!(foreign_keywords(&plugin), Vec::<String>::new());
}

#[test]
fn a_method_without_a_value_param_is_unchanged() {
    let get: Value = serde_json::to_value(method("get")).unwrap();
    assert!(!get.to_string().contains("\"null\",\"boolean\""), "{get}");
}
