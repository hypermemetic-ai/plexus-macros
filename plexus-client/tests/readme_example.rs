//! The README's example: a call through a `Transport`, its items decoded,
//! and a call the server refuses.

use futures::TryStreamExt;
use plexus_client::{decode, ClientError, MemoryTransport, MethodRef, RemoteError, StreamItem, Transport};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Debug, PartialEq, Serialize, Deserialize)]
struct Tick {
    n: u32,
}

const COUNT: MethodRef = MethodRef::new("counter", "count");

#[test]
fn a_call_is_a_stream_of_its_items() {
    let transport = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }, Tick { n: 2 }]);

    let items = decode::<Tick>(transport.call(COUNT, json!({ "to": 2 })));
    let ticks: Vec<Tick> = futures::executor::block_on(items.try_collect()).unwrap();

    assert_eq!(ticks, [Tick { n: 1 }, Tick { n: 2 }]);
    assert_eq!(transport.params_of(COUNT), Some(json!({ "to": 2 })));
}

#[test]
fn a_refused_call_is_a_typed_error() {
    let refusal = json!({ "type": "error", "error": { "kind": "unauthenticated" }, "message": "sign in" });
    let refusal: StreamItem = serde_json::from_value(refusal).unwrap();
    let transport = MemoryTransport::new().script(COUNT, vec![refusal]);

    let items = decode::<Tick>(transport.call(COUNT, json!({ "to": 2 })));
    let refused = futures::executor::block_on(items.try_collect::<Vec<Tick>>()).unwrap_err();

    assert!(matches!(refused, ClientError::Remote(RemoteError::Unauthenticated)));
}
