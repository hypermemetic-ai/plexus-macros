//! RETURNS-40: the schemas the fixture's services answer are the ones its
//! `expected/` holds — what synapse builds its IR from in `ir3-test`.
//! `IR_FIXTURE_ACCEPT=1` writes them instead.

mod support;

use support::*;

async fn check(hub: plexus_core::plexus::DynamicHub, file: &str) {
    let served = serde_json::to_value(walk(&hub).await).unwrap();
    if accept() {
        write(file, &pretty(&served));
        return;
    }
    let expected: serde_json::Value = serde_json::from_str(&read(file)).unwrap();
    assert!(served == expected, "{file}: the served schemas moved (IR_FIXTURE_ACCEPT=1 writes them; then regenerate the IR in synapse)");
}

#[tokio::test]
async fn the_services_schemas_are_the_fixtures() {
    check(ir_service::service(), "schemas.json").await;
}

#[tokio::test]
async fn the_clash_services_schemas_are_the_fixtures() {
    check(ir_service::clash_service(), "clash.schemas.json").await;
}
