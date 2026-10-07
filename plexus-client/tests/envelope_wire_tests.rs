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
//! `tests/client_envelope_compat.rs` over in `plexus-core` closes the loop
//! by round-tripping items serialized by plexus-core itself, byte for byte.

use plexus_client::{DenyReason, Note, RemoteError, StreamItem, StreamMetadata};
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

/// The 0.7 error frame: `recoverable` and a typed `RemoteError` (RETURNS-6).
/// Round-trips byte-for-byte when every field is present.
#[test]
fn websocket_error_frame_carries_a_typed_remote_error() {
    let wire = json!({
        "type": "error",
        "metadata": {"provenance": ["menger"], "plexus_hash": "h", "timestamp": 1i64},
        "recoverable": false,
        "error": {"kind": "forbidden", "reason": "missing_scope", "scope": "tickets:write"}
    });
    let item: StreamItem = serde_json::from_value(wire.clone()).expect("deserializes");
    match &item {
        StreamItem::Error {
            error, recoverable, ..
        } => {
            assert_eq!(
                error,
                &RemoteError::Forbidden {
                    reason: DenyReason::MissingScope {
                        scope: "tickets:write".into()
                    }
                }
            );
            assert!(!recoverable);
        }
        other => panic!("expected error, got {other:?}"),
    }
    assert!(item.is_terminal());
    assert_eq!(serde_json::to_value(&item).unwrap(), wire);
}

/// A transport that reassembles an error frame from a body with no
/// `recoverable` (a REST unary body carries only the `RemoteError`) still
/// reads: `recoverable` defaults to `false`.
#[test]
fn an_error_frame_without_recoverable_defaults_it() {
    let item: StreamItem = serde_json::from_value(
        json!({"type": "error", "error": {"kind": "execution", "message": "boom"}}),
    )
    .expect("deserializes");
    match item {
        StreamItem::Error {
            error,
            recoverable,
            metadata,
        } => {
            assert_eq!(error, RemoteError::Execution { message: "boom".into() });
            assert!(!recoverable, "defaults to not-recoverable");
            assert!(metadata.is_none());
        }
        other => panic!("expected error, got {other:?}"),
    }
}

/// No back-compat (RETURNS-1 decision 12): a 0.6 error frame — `message` /
/// `code`, no typed `error` — does not read.
#[test]
fn a_06_error_frame_does_not_read() {
    let old = json!({"type": "error", "message": "boom", "code": "E1"});
    assert!(serde_json::from_value::<StreamItem>(old).is_err());
}

/// plexus-transport's `event: done` payload is literally `{}`.
#[test]
fn sse_done_frame_is_an_empty_object() {
    let item: StreamItem =
        serde_json::from_value(json!({"type": "done"})).expect("deserializes");
    assert!(matches!(item, StreamItem::Done { metadata: None }));
    assert!(item.is_terminal());
}

/// The 0.7 progress frame: `content_type` and the method's progress value
/// as `content` (RETURNS-6). Round-trips byte-for-byte.
#[test]
fn websocket_progress_frame_carries_typed_content() {
    let wire = json!({
        "type": "progress",
        "metadata": {"provenance": ["menger"], "plexus_hash": "h", "timestamp": 1i64},
        "content_type": "tickets.import",
        "content": {"type": "chunk", "done": 0, "of": 10}
    });
    let item: StreamItem = serde_json::from_value(wire.clone()).expect("deserializes");
    match &item {
        StreamItem::Progress {
            content_type,
            content,
            ..
        } => {
            assert_eq!(content_type.as_deref(), Some("tickets.import"));
            assert_eq!(content["of"], 10);
        }
        other => panic!("expected progress, got {other:?}"),
    }
    assert!(!item.is_terminal());
    assert_eq!(serde_json::to_value(&item).unwrap(), wire);
}

/// The default progress type is a `Note`, `percentage` optional; a progress
/// frame need not carry `metadata` or `content_type`.
#[test]
fn progress_content_is_a_note_by_default() {
    let item: StreamItem = serde_json::from_value(
        json!({"type": "progress", "content": {"message": "m", "percentage": 50.0}}),
    )
    .unwrap();
    match item {
        StreamItem::Progress {
            metadata: None,
            content_type: None,
            content,
        } => {
            let note: Note = serde_json::from_value(content).unwrap();
            assert_eq!(note, Note::new("m").with_percentage(50.0));
        }
        other => panic!("expected progress, got {other:?}"),
    }

    // 0.6's `{message, percentage}` at the top level is not a 0.7 frame.
    assert!(serde_json::from_value::<StreamItem>(json!({"type": "progress", "message": "m"})).is_err());
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
            content_type: None,
            content: json!({"message": "m"}),
        }),
        "progress"
    );
    assert_eq!(
        tag(&StreamItem::Error {
            metadata: None,
            recoverable: false,
            error: RemoteError::Unauthenticated,
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
