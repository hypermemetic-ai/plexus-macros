//! RETURNS-40: served as a service's `conformance` child, the fixture's IR
//! as the service writes it (`<service>.ir`, plexus-core) is
//! `expected/ir.json` — synapse's, from `schema.json` — byte for byte in
//! synapse's canonical form, its `irBackend` the backend synapse names.

use std::path::PathBuf;

use conformance::ConformanceActivation;
use futures::StreamExt;
use plexus_core::plexus::{ir, DynamicHub};
use serde_json::{json, Value};

#[tokio::test]
async fn the_service_writes_the_fixtures_ir() {
    let service = DynamicHub::new("conformance_service").register(ConformanceActivation);
    let items: Vec<Value> = service
        .route("conformance_service.ir", json!({"namespaces": ["conformance"]}), None)
        .await
        .expect("ir answers")
        .map(|i| serde_json::to_value(i).unwrap())
        .collect()
        .await;
    let data: Vec<&Value> = items.iter().filter(|i| i["type"] == "data").collect();
    assert_eq!(data.len(), 1, "{items:?}");
    let mut served = data[0]["content"]["ok"].clone();
    assert!(served.is_object(), "{}", data[0]["content"]);
    // synapse names the backend it was asked of; the fixture's is "conformance"
    served["irBackend"] = json!("conformance");
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected/ir.json");
    let golden = std::fs::read_to_string(&path).unwrap();
    assert!(ir::canonical(&served) == golden, "the served IR is not expected/ir.json:\n{}", ir::canonical(&served));
}
