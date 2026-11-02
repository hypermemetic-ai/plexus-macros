//! RETURNS-40: the IR the service writes (`<service>.ir`) is the IR synapse
//! builds from the same service's raw schemas — `expected/ir.json` and
//! `expected/ir.alpha.json`, written by synapse's `ir3-test` from
//! `expected/schemas.json` — byte for byte in synapse's canonical form. A
//! title collision is refused as synapse refuses it (`clash.error.txt`).

mod support;

use serde_json::{json, Value};
use support::*;

/// The IR an `ir` call answers, `irBackend` as synapse names the backend.
async fn ir(hub: &plexus_core::plexus::DynamicHub, params: Value) -> Result<Value, Value> {
    let method = format!("{}.ir", hub.runtime_namespace());
    let answer = content(&call(hub, &method, params).await);
    match (answer.get("ok"), answer.get("err")) {
        (Some(ok), None) => Ok(ok.clone()),
        (None, Some(err)) => Err(err.clone()),
        _ => panic!("ir answers ok or err: {answer}"),
    }
}

fn same_as(file: &str, served: &Value) {
    let golden = read(file);
    // synapse's canonical form: keys sorted, two-space indent, a trailing newline
    let mine = plexus_core::plexus::ir::canonical(served);
    assert!(mine == golden, "{file}: the service's IR is not synapse's\n--- service ---\n{mine}");
}

#[tokio::test]
async fn the_whole_service() {
    let hub = ir_service::service();
    let served = ir(&hub, json!({})).await.expect("the service's IR");
    same_as("ir.json", &served);
    assert!(!served["irRefusals"].as_array().map(Vec::is_empty).unwrap_or(true), "the reader's refusals are carried");
}

#[tokio::test]
async fn none_named_is_all() {
    let hub = ir_service::service();
    assert_eq!(ir(&hub, json!({"namespaces": []})).await, ir(&hub, json!({})).await);
}

#[tokio::test]
async fn one_subtree() {
    let hub = ir_service::service();
    let served = ir(&hub, json!({"namespaces": ["alpha"]})).await.expect("alpha's IR");
    same_as("ir.alpha.json", &served);
}

#[tokio::test]
async fn a_title_collision_is_refused_as_synapse_refuses_it() {
    let hub = ir_service::clash_service();
    let err = ir(&hub, json!({})).await.expect_err("two shapes under one title");
    assert_eq!(err["type"], "title_collision", "{err}");
    assert_eq!(err["message"].as_str().unwrap(), read("clash.error.txt").trim_end(), "{err}");
}

#[tokio::test]
async fn a_namespace_the_service_has_not_is_refused() {
    let hub = ir_service::service();
    let err = ir(&hub, json!({"namespaces": ["delta"]})).await.expect_err("no delta");
    assert_eq!(err["type"], "not_found", "{err}");
}

/// Asked twice, answered the same — the second from what the service kept.
#[tokio::test]
async fn asked_again_the_same() {
    let hub = ir_service::service();
    let a = ir(&hub, json!({"namespaces": ["beta"]})).await;
    let b = ir(&hub, json!({"namespaces": ["beta"]})).await;
    assert_eq!(a, b);
}
