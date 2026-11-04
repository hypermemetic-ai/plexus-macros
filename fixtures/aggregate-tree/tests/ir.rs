//! RETURNS-46: the tree's IR 3.0, as the service writes it (`<service>.ir`,
//! plexus-core's, equal to synapse's), is `expected/ir.json` — what
//! service-codegen's TypeScript aggregate is generated from
//! (`service-codegen/tests/fixtures/returns46_tree_ir.json` is a copy).
//! `AGGREGATE_TREE_ACCEPT=1` rewrites the file from the service.

use std::path::PathBuf;

use aggregate_tree::TreeActivation;
use futures::StreamExt;
use plexus_core::plexus::{ir, DynamicHub};
use serde_json::{json, Value};

#[tokio::test]
async fn the_service_writes_the_trees_ir() {
    let service = DynamicHub::new("tree_service").register(TreeActivation);
    let items: Vec<Value> = service
        .route("tree_service.ir", json!({"namespaces": ["tree"]}), None)
        .await
        .expect("ir answers")
        .map(|i| serde_json::to_value(i).unwrap())
        .collect()
        .await;
    let data: Vec<&Value> = items.iter().filter(|i| i["type"] == "data").collect();
    assert_eq!(data.len(), 1, "{items:?}");
    let mut served = data[0]["content"]["ok"].clone();
    assert!(served.is_object(), "{}", data[0]["content"]);
    served["irBackend"] = json!("tree");
    let served = ir::canonical(&served);
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected/ir.json");
    if std::env::var("AGGREGATE_TREE_ACCEPT").is_ok() {
        std::fs::write(&path, &served).unwrap();
    }
    let golden = std::fs::read_to_string(&path).unwrap();
    assert!(served == golden, "the served IR is not expected/ir.json:\n{served}");
}
