//! RETURNS-46 review: `#[child(opaque)]` — a dynamic child with no client
//! (substrate's `SessionChild`) and a hand-written `ChildRouter` compile with
//! that one attribute, get no accessor and no variant, and route as before.

use std::sync::Arc;

use aggregate_probe::opaque::{HostActivation, HostClient, HostEvent, RawHostActivation};
use futures::StreamExt;
use plexus_client::{MethodRef, StreamItem, Transport};
use plexus_core::plexus::{DynamicHub, HubTransport};
use serde_json::{json, Value};

async fn data(t: &HubTransport, ns: &'static str, method: &'static str, path: &'static str) -> Vec<Value> {
    t.call(MethodRef::new(ns, method).at(path), json!({ "n": 1 }))
        .filter_map(|i| async move {
            match i.unwrap() {
                StreamItem::Data { content, .. } => Some(content),
                StreamItem::Error { error, .. } => panic!("{error:?}"),
                _ => None,
            }
        })
        .collect()
        .await
}

#[tokio::test]
async fn an_opaque_childs_instances_are_routed_as_before() {
    let t = HubTransport::anonymous(Arc::new(DynamicHub::new("svc").register(HostActivation)));
    assert_eq!(data(&t, "session", "tick", "host.abc").await, [json!({ "n": 300 })]);
    // the parent's own client and aggregate know nothing of it
    let ev: Vec<HostEvent> =
        futures::TryStreamExt::try_collect(HostClient::new(&t).tick(1).events()).await.unwrap();
    assert_eq!(serde_json::to_string(&ev[0]).unwrap(), r#"{"method":"tick","event":{"n":0}}"#);
}

#[tokio::test]
async fn a_hand_written_router_as_an_opaque_child() {
    let t = HubTransport::anonymous(Arc::new(DynamicHub::new("svc").register(RawHostActivation)));
    assert_eq!(data(&t, "raw", "hello", "raw_host.zed").await, [json!({ "hello": "zed" })]);
}

#[test]
fn an_opaque_child_is_no_variant() {
    // exhaustive: tick alone
    fn only(e: HostEvent) {
        match e {
            HostEvent::Tick(_) => {}
        }
    }
    only(HostEvent::Tick(aggregate_probe::N { n: 0 }));
}
