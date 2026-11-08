//! RETURNS-40: the IR the service writes (`<service>.ir`) is the IR synapse
//! builds from the same service's raw schemas — `expected/ir.json`,
//! `expected/ir.alpha.json` and `expected/clash.ir.json`, written by synapse's `ir3-test` from
//! `expected/schemas.json` — byte for byte in synapse's canonical form. A
//! title collision refuses its activation, as synapse's walk does.

mod support;

use serde_json::{json, Value};
use support::*;

/// The IR an `ir` call answers, `irBackend` as synapse names the backend.
async fn ir(service: &plexus_core::plexus::DynamicHub, params: Value) -> Result<Value, Value> {
    let method = format!("{}.ir", service.runtime_namespace());
    let answer = content(&call(service, &method, params).await);
    match (answer.get("ok"), answer.get("err")) {
        (Some(ok), None) => Ok(ok.clone()),
        (None, Some(err)) => Err(err.clone()),
        _ => panic!("ir answers ok or err: {answer}"),
    }
}

fn same_as(file: &str, served: &Value) {
    // synapse's canonical form: keys sorted, two-space indent, a trailing newline
    let mine = plexus_core::plexus::ir::canonical(served);
    // ASK-2: `IR_FIXTURE_ACCEPT=1` writes it too (plexus-core's IR is held
    // equal to synapse's: regenerate synapse's golden from the same schemas).
    if accept() {
        write(file, &mine);
    }
    let golden = read(file);
    assert!(mine == golden, "{file}: the service's IR is not synapse's\n--- service ---\n{mine}");
}

#[tokio::test]
async fn the_whole_service() {
    let service = ir_service::service();
    let served = ir(&service, json!({})).await.expect("the service's IR");
    same_as("ir.json", &served);
    assert!(!served["irRefusals"].as_array().map(Vec::is_empty).unwrap_or(true), "the reader's refusals are carried");
}

#[tokio::test]
async fn none_named_is_all() {
    let service = ir_service::service();
    assert_eq!(ir(&service, json!({"namespaces": []})).await, ir(&service, json!({})).await);
}

#[tokio::test]
async fn one_subtree() {
    let service = ir_service::service();
    let served = ir(&service, json!({"namespaces": ["alpha"]})).await.expect("alpha's IR");
    same_as("ir.alpha.json", &served);
}

/// RETURNS-36 review #9: a title collision refuses its activation, not the
/// walk — the clash service's IR is synapse's, the `clash` activation left
/// out with a `title` refusal naming both sites.
#[tokio::test]
async fn a_title_collision_refuses_its_activation_as_synapse_does() {
    let service = ir_service::clash_service();
    let served = ir(&service, json!({})).await.expect("the walk goes on");
    same_as("clash.ir.json", &served);
    let titles: Vec<&Value> = served["irRefusals"].as_array().unwrap().iter().filter(|r| r["rfKeyword"] == "title").collect();
    assert_eq!(titles.len(), 1, "{}", served["irRefusals"]);
    assert_eq!(titles[0]["rfSite"], "clash");
}

#[tokio::test]
async fn a_namespace_the_service_has_not_is_refused() {
    // RETURNS-40 review #1: a call failure, invalid_params naming it, before any work
    let service = ir_service::service();
    let items = call(&service, "irsvc.ir", json!({"namespaces": ["delta"]})).await;
    assert_eq!(items[0]["type"], "error", "{items:?}");
    assert_eq!(items[0]["error"]["kind"], "invalid_params", "{items:?}");
    assert!(items[0]["message"].as_str().unwrap().contains("delta"), "{items:?}");
    assert_eq!(service.ir_builds(), 0);
}

/// Asked twice, answered the same — the second from what the service kept.
#[tokio::test]
async fn asked_again_the_same() {
    let service = ir_service::service();
    let a = ir(&service, json!({"namespaces": ["beta"]})).await;
    let b = ir(&service, json!({"namespaces": ["beta"]})).await;
    assert_eq!(a, b);
}
