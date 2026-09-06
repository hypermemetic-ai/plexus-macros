//! The envelope against the two wire shapes it must accept.
//!
//! Every literal below is copied from a real frame:
//!
//! - the WebSocket shape from `plexus_core::plexus::types::PlexusStreamItem`
//!   (`#[serde(tag = "type", rename_all = "snake_case")]`), as asserted in
//!   menger's `tests/wire.rs` (`item["type"]`, `item["content"]`);
//! - the REST + SSE shape from plexus-transport's `stream_sse_response`,
//!   which emits `event: data` / `data: {"content": …}` and drops
//!   `metadata` and `content_type`.
//!
//! `tests/client_envelope_compat.rs` over in `plexus-macros` closes the loop
//! by round-tripping items serialized by plexus-core itself.

use plexus_client::{StreamItem, StreamMetadata};
use serde_json::json;

/// The full WebSocket JSON-RPC envelope — every field present.
#[test]
fn websocket_data_item_deserializes_losslessly() {
    let wire = json!({
        "type": "data",
        "metadata": {
            "provenance": ["menger", "tickets"],
            "plexus_hash": "abc123",
            "timestamp": 1735052400i64
        },
        "content_type": "tickets.watch",
        "content": {"type": "created", "ticket": {"id": "WIRE-1"}}
    });

    let item: StreamItem = serde_json::from_value(wire.clone()).expect("deserializes");
    match &item {
        StreamItem::Data {
            metadata,
            content_type,
            content,
        } => {
            assert_eq!(
                metadata.as_ref().map(|m| m.provenance.as_slice()),
                Some(["menger".to_string(), "tickets".to_string()].as_slice())
            );
            assert_eq!(content_type.as_deref(), Some("tickets.watch"));
            assert_eq!(content["ticket"]["id"], "WIRE-1");
        }
        other => panic!("expected data, got {other:?}"),
    }

    // Round-trips byte-for-byte when every field is present.
    assert_eq!(serde_json::to_value(&item).unwrap(), wire);
}

/// The REST + SSE shape: `metadata` and `content_type` were never sent.
/// A client type that required them could not represent this at all.
#[test]
fn sse_shaped_data_item_deserializes_without_metadata() {
    let reassembled = json!({
        "type": "data",
        "content": {"type": "created", "ticket": {"id": "RST-1"}}
    });

    let item: StreamItem = serde_json::from_value(reassembled).expect("deserializes");
    assert!(matches!(
        item,
        StreamItem::Data {
            metadata: None,
            content_type: None,
            ..
        }
    ));
    assert!(item.metadata().is_none());
}

/// plexus-transport's `event: error` payload is `{"message", "code"}` only —
/// `recoverable` is not on the wire and must default.
#[test]
fn sse_error_frame_has_no_recoverable_field() {
    let item: StreamItem =
        serde_json::from_value(json!({"type": "error", "message": "boom", "code": "E1"}))
            .expect("deserializes");
    match item {
        StreamItem::Error {
            message,
            code,
            recoverable,
            metadata,
        } => {
            assert_eq!(message, "boom");
            assert_eq!(code.as_deref(), Some("E1"));
            assert!(!recoverable, "defaults to not-recoverable");
            assert!(metadata.is_none());
        }
        other => panic!("expected error, got {other:?}"),
    }
}

/// plexus-transport's `event: done` payload is literally `{}`.
#[test]
fn sse_done_frame_is_an_empty_object() {
    let item: StreamItem =
        serde_json::from_value(json!({"type": "done"})).expect("deserializes");
    assert!(matches!(item, StreamItem::Done { metadata: None }));
    assert!(item.is_terminal());
}

#[test]
fn progress_percentage_is_optional_on_both_transports() {
    let with: StreamItem =
        serde_json::from_value(json!({"type": "progress", "message": "m", "percentage": 50.0}))
            .unwrap();
    assert!(matches!(
        with,
        StreamItem::Progress {
            percentage: Some(p),
            ..
        } if (p - 50.0).abs() < f32::EPSILON
    ));

    let without: StreamItem =
        serde_json::from_value(json!({"type": "progress", "message": "m"})).unwrap();
    assert!(matches!(
        without,
        StreamItem::Progress {
            percentage: None,
            ..
        }
    ));
    assert!(!without.is_terminal());
}

/// The `request` variant keeps its camelCase field names — it is the one
/// variant plexus-core renames differently.
#[test]
fn request_variant_is_camel_case_on_the_wire() {
    let item: StreamItem = serde_json::from_value(json!({
        "type": "request",
        "requestId": "r-1",
        "requestData": {"prompt": "continue?"},
        "timeoutMs": 30000
    }))
    .expect("deserializes");

    assert!(matches!(
        item,
        StreamItem::Request { ref request_id, timeout_ms, .. }
            if request_id == "r-1" && timeout_ms == 30000
    ));
}

/// Variant tags are snake_case, matching plexus-core. (plexus-codegen-rust's
/// hand-maintained copy says `rename_all = "lowercase"` — indistinguishable
/// only because every variant happens to be one word. Pinning it here is the
/// point.)
#[test]
fn variant_tags_are_snake_case() {
    let tag = |i: &StreamItem| serde_json::to_value(i).unwrap()["type"].as_str().unwrap().to_string();

    assert_eq!(tag(&StreamItem::data(json!({}))), "data");
    assert_eq!(tag(&StreamItem::done()), "done");
    assert_eq!(
        tag(&StreamItem::Progress {
            metadata: None,
            message: "m".into(),
            percentage: None
        }),
        "progress"
    );
    assert_eq!(
        tag(&StreamItem::Error {
            metadata: None,
            message: "m".into(),
            code: None,
            recoverable: false
        }),
        "error"
    );
    assert_eq!(
        tag(&StreamItem::Request {
            request_id: "r".into(),
            request_data: json!({}),
            timeout_ms: 0
        }),
        "request"
    );
}

#[test]
fn absent_metadata_is_omitted_not_nulled() {
    let out = serde_json::to_value(StreamItem::done()).unwrap();
    assert_eq!(out, json!({"type": "done"}), "no `metadata: null` key");
}

#[test]
fn stream_metadata_tolerates_a_partial_object() {
    let m: StreamMetadata = serde_json::from_value(json!({"plexus_hash": "h"})).unwrap();
    assert_eq!(m.plexus_hash, "h");
    assert!(m.provenance.is_empty());
    assert_eq!(m.timestamp, 0);
}
