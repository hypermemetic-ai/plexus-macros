//! What both tests do: serve a call through the service as a socket's
//! `<service>.call` does, and walk its schemas as synapse does.

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use futures::StreamExt;
use plexus_core::plexus::{DynamicHub, PlexusStreamItem};
use serde_json::{json, Value};

pub fn expected_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected")
}

pub fn read(rel: &str) -> String {
    let path = expected_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// Whether `IR_FIXTURE_ACCEPT=1`: write what the service answers, rather
/// than compare.
pub fn accept() -> bool {
    std::env::var("IR_FIXTURE_ACCEPT").as_deref() == Ok("1")
}

pub fn write(rel: &str, text: &str) {
    let path = expected_dir().join(rel);
    std::fs::write(&path, text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
}

/// The items one call answers, as JSON — an error the route refuses with
/// as the one error item a socket would be sent.
pub async fn call(service: &DynamicHub, method: &str, params: Value) -> Vec<Value> {
    match service.route(method, params, None).await {
        Ok(stream) => stream.map(|item: PlexusStreamItem| serde_json::to_value(item).expect("an item serializes")).collect().await,
        Err(e) => vec![plexus_core::plexus::error_item(service.runtime_namespace(), &e)],
    }
}

/// The content of the one `data` item a call answers.
pub fn content(items: &[Value]) -> Value {
    let data: Vec<&Value> = items.iter().filter(|i| i["type"] == "data").collect();
    assert_eq!(data.len(), 1, "one data item: {items:?}");
    data[0]["content"].clone()
}

/// Every schema synapse's walk reads, by dotted path (`""`: the root):
/// `<service>.schema`, then each child summary's `<path>.schema`.
pub async fn walk(service: &DynamicHub) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    let mut todo = vec![String::new()];
    while let Some(path) = todo.pop() {
        let method = if path.is_empty() { format!("{}.schema", service.runtime_namespace()) } else { format!("{path}.schema") };
        let schema = content(&call(service, &method, json!({})).await);
        if let Some(kids) = schema.get("children").and_then(Value::as_array) {
            for k in kids {
                let ns = k["namespace"].as_str().expect("a child names its namespace");
                todo.push(if path.is_empty() { ns.to_string() } else { format!("{path}.{ns}") });
            }
        }
        out.insert(path, schema);
    }
    out
}

/// JSON pretty-printed, two-space indent, one trailing newline — for the
/// schemas, which are compared parsed.
pub fn pretty(v: &Value) -> String {
    serde_json::to_string_pretty(v).expect("JSON") + "\n"
}
