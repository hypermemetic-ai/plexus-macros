//! The decode contract: every wire item shape, and what the typed stream
//! does with it. These are the assertions a transport implementation is
//! written against.

use futures_util::StreamExt;
use plexus_client::{
    collect_data, decode, ClientError, ClientEvent, MemoryTransport, MethodRef, RemoteError,
    StreamItem, StreamMetadata, Transport, TransportError, TransportErrorKind,
};
use serde::{Deserialize, Serialize};
use serde_json::json;

/// Stands in for an activation's event enum. Externally tagged the same way
/// `TicketsEvent` is (`#[serde(tag = "type", rename_all = "snake_case")]`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum DemoEvent {
    Health { version: String, count: u64 },
    Item { id: String },
    Error { message: String },
}

const HEALTH: MethodRef = MethodRef::new("demo", "health");
const WATCH: MethodRef = MethodRef::new("demo", "watch");

#[tokio::test]
async fn data_items_decode_into_the_activations_own_event_type() {
    let t = MemoryTransport::new().script_events(
        HEALTH,
        &[
            DemoEvent::Health {
                version: "0.6.0".into(),
                count: 3,
            },
            DemoEvent::Item { id: "A-1".into() },
        ],
    );

    let events = collect_data(decode::<DemoEvent>(t.call(HEALTH, json!({}))))
        .await
        .expect("no error");

    assert_eq!(
        events,
        vec![
            DemoEvent::Health {
                version: "0.6.0".into(),
                count: 3
            },
            DemoEvent::Item { id: "A-1".into() },
        ]
    );
}

#[tokio::test]
async fn done_ends_the_stream_and_is_not_itself_an_event() {
    let t = MemoryTransport::new().script(
        HEALTH,
        vec![
            StreamItem::data(json!({"type": "item", "id": "A-1"})),
            StreamItem::done(),
        ],
    );

    let got: Vec<_> = decode::<DemoEvent>(t.call(HEALTH, json!({}))).collect().await;
    assert_eq!(got.len(), 1, "done is termination, not an event: {got:?}");
    assert!(matches!(got[0], Ok(ClientEvent::Data(_))));
}

#[tokio::test]
async fn progress_is_surfaced_and_does_not_terminate() {
    let t = MemoryTransport::new().script(
        WATCH,
        vec![
            StreamItem::Progress {
                metadata: None,
                content_type: Some("demo.watch".into()),
                content: json!({"message": "scanning", "percentage": 25.0}),
            },
            StreamItem::data(json!({"type": "item", "id": "A-1"})),
            StreamItem::done(),
        ],
    );

    let got: Vec<_> = decode::<DemoEvent>(t.call(WATCH, json!({}))).collect().await;
    assert_eq!(got.len(), 2);
    assert_eq!(
        got[0].as_ref().unwrap(),
        &ClientEvent::Progress {
            message: "scanning".into(),
            percentage: Some(25.0)
        }
    );
    assert!(matches!(got[1], Ok(ClientEvent::Data(_))));
}

/// A method's own typed progress value (not a `Note`) still surfaces, as
/// its JSON text, until the typed client decodes it (RETURNS-9).
#[tokio::test]
async fn typed_progress_surfaces_as_its_json_text() {
    let t = MemoryTransport::new().script(
        WATCH,
        vec![
            StreamItem::Progress {
                metadata: None,
                content_type: Some("demo.watch".into()),
                content: json!({"type": "chunk", "done": 1, "of": 4}),
            },
            StreamItem::done(),
        ],
    );

    let got: Vec<_> = decode::<DemoEvent>(t.call(WATCH, json!({}))).collect().await;
    assert_eq!(got.len(), 1, "{got:?}");
    match &got[0] {
        Ok(ClientEvent::Progress {
            message,
            percentage: None,
        }) => {
            let text: serde_json::Value = serde_json::from_str(message).expect("JSON text");
            assert_eq!(text, json!({"type": "chunk", "done": 1, "of": 4}));
        }
        other => panic!("expected progress, got {other:?}"),
    }
}

/// The requirement the ticket words as "transport errors and in-band `error`
/// items are distinguishable in the returned stream".
#[tokio::test]
async fn in_band_error_and_transport_error_are_different_types() {
    let in_band = MemoryTransport::new().script(
        HEALTH,
        vec![StreamItem::Error {
            metadata: Some(StreamMetadata::default()),
            recoverable: false,
            error: RemoteError::MethodNotFound {
                activation: "demo".into(),
                method: "nope".into(),
            },
        }],
    );
    let err = collect_data(decode::<DemoEvent>(in_band.call(HEALTH, json!({}))))
        .await
        .unwrap_err();
    assert!(err.is_remote(), "{err:?}");
    assert!(!err.is_transport());
    // Until RETURNS-9's `ClientError::Remote(RemoteError)`: the error
    // frame's `Display` line and `kind` tag.
    assert_eq!(
        err,
        ClientError::Remote {
            message: "method not found: demo.nope".into(),
            code: Some("method_not_found".into()),
            recoverable: false,
        }
    );

    let dropped =
        MemoryTransport::new().script_failure(HEALTH, TransportError::connect("connection refused"));
    let err = collect_data(decode::<DemoEvent>(dropped.call(HEALTH, json!({}))))
        .await
        .unwrap_err();
    assert!(err.is_transport(), "{err:?}");
    assert!(!err.is_remote());
    assert!(matches!(
        err,
        ClientError::Transport(TransportError {
            kind: TransportErrorKind::Connect,
            ..
        })
    ));
}

/// An activation's *own* `error` variant is a data event, not a client
/// error — the two error depths menger's vanilla client distinguishes by
/// hand are distinguished here by type.
#[tokio::test]
async fn an_activation_error_variant_is_still_a_data_event() {
    let t = MemoryTransport::new()
        .script_events(HEALTH, &[DemoEvent::Error { message: "nope".into() }]);

    let events = collect_data(decode::<DemoEvent>(t.call(HEALTH, json!({}))))
        .await
        .expect("in-activation errors are data, not Err");
    assert_eq!(events, vec![DemoEvent::Error { message: "nope".into() }]);
}

#[tokio::test]
async fn a_data_item_that_does_not_match_the_event_type_is_a_decode_error() {
    let t = MemoryTransport::new().script(
        HEALTH,
        vec![
            StreamItem::Data {
                metadata: None,
                content_type: Some("demo.health".into()),
                content: json!({"type": "health", "version": "0.6.0"}), // `count` missing
            },
            StreamItem::done(),
        ],
    );

    let err = collect_data(decode::<DemoEvent>(t.call(HEALTH, json!({}))))
        .await
        .unwrap_err();
    assert!(matches!(
        err,
        ClientError::Decode { ref content_type, .. }
            if content_type.as_deref() == Some("demo.health")
    ));
}

#[tokio::test]
async fn a_terminal_error_stops_the_stream() {
    let t = MemoryTransport::new().script(
        WATCH,
        vec![
            StreamItem::data(json!({"type": "item", "id": "A-1"})),
            StreamItem::Error {
                metadata: None,
                recoverable: false,
                error: RemoteError::Execution {
                    message: "boom".into(),
                },
            },
            // Anything after a terminal item must not be delivered.
            StreamItem::data(json!({"type": "item", "id": "A-2"})),
        ],
    );

    let got: Vec<_> = decode::<DemoEvent>(t.call(WATCH, json!({}))).collect().await;
    assert_eq!(got.len(), 2, "{got:?}");
    assert!(got[0].is_ok());
    assert!(got[1].as_ref().unwrap_err().is_remote());
}

#[tokio::test]
async fn bidirectional_request_items_are_skipped_not_fatal() {
    let t = MemoryTransport::new().script(
        WATCH,
        vec![
            StreamItem::Request {
                request_id: "r1".into(),
                request_data: json!({"prompt": "continue?"}),
                timeout_ms: 1000,
            },
            StreamItem::data(json!({"type": "item", "id": "A-1"})),
            StreamItem::done(),
        ],
    );

    let events = collect_data(decode::<DemoEvent>(t.call(WATCH, json!({}))))
        .await
        .expect("request items are skipped");
    assert_eq!(events, vec![DemoEvent::Item { id: "A-1".into() }]);
}

#[tokio::test]
async fn an_unscripted_method_is_a_transport_protocol_error() {
    let t = MemoryTransport::new();
    let err = collect_data(decode::<DemoEvent>(t.call(HEALTH, json!({}))))
        .await
        .unwrap_err();
    assert!(err.is_transport());
}

#[tokio::test]
async fn the_transport_records_the_params_it_was_handed() {
    let t = MemoryTransport::new().script_events(WATCH, &[DemoEvent::Item { id: "A-1".into() }]);
    let _ = collect_data(decode::<DemoEvent>(
        t.call(WATCH, json!({"epic": "MACRO"})),
    ))
    .await;

    assert_eq!(t.calls(), vec![("demo.watch".to_string(), json!({"epic": "MACRO"}))]);
    assert_eq!(t.params_of(WATCH), Some(json!({"epic": "MACRO"})));
}

#[test]
fn method_ref_keeps_the_namespace_and_the_method_separable() {
    assert_eq!(WATCH.namespace(), "demo");
    assert_eq!(WATCH.method(), "watch");
    assert_eq!(WATCH.qualified(), "demo.watch");
    assert_eq!(WATCH.to_string(), "demo.watch");
}
