//! MACRO-2 · the drift gate between the server envelope and the client one.
//!
//! `plexus_client::StreamItem` is a deliberately *looser* mirror of
//! `plexus_core::plexus::PlexusStreamItem` — it has to be, because the REST
//! bridge drops `metadata` and `content_type` on the floor (see
//! `plexus-client/src/envelope.rs`). Looser is not the same as *different*,
//! and "two hand-maintained copies of the wire type" is precisely the drift
//! this whole ticket exists to remove.
//!
//! There is prior art for getting this wrong: `plexus-codegen-rust`'s
//! `src/types.rs` carries a third copy of the same enum with
//! `rename_all = "lowercase"` (vs plexus-core's `snake_case`) and
//! `percentage: f64` (vs `f32`). Nothing catches that today.
//!
//! So this test is the catch: it serializes real `PlexusStreamItem`s with
//! plexus-core's own `Serialize` impl and asserts the client type reads
//! every variant back with the same meaning. This crate can host it because
//! it already dev-depends on both.

use plexus_client::StreamItem as ClientItem;
use plexus_core::plexus::{PlexusStreamItem as ServerItem, StreamMetadata as ServerMetadata};
use serde_json::json;

fn meta() -> ServerMetadata {
    ServerMetadata {
        provenance: vec!["menger".into(), "tickets".into()],
        plexus_hash: "abc123".into(),
        timestamp: 1_735_052_400,
    }
}

/// Serialize with the server's impl, deserialize with the client's.
fn cross(server: &ServerItem) -> ClientItem {
    let wire = serde_json::to_value(server).expect("server serializes");
    serde_json::from_value(wire.clone())
        .unwrap_or_else(|e| panic!("client cannot read server wire {wire}: {e}"))
}

#[test]
fn data_variant_survives_the_crossing() {
    let server = ServerItem::data(
        meta(),
        "tickets.watch".into(),
        json!({"type": "created", "ticket": {"id": "WIRE-1"}}),
    );

    match cross(&server) {
        ClientItem::Data {
            metadata,
            content_type,
            content,
        } => {
            let m = metadata.expect("websocket transport carries metadata");
            assert_eq!(m.provenance, vec!["menger".to_string(), "tickets".to_string()]);
            assert_eq!(m.plexus_hash, "abc123");
            assert_eq!(m.timestamp, 1_735_052_400);
            assert_eq!(content_type.as_deref(), Some("tickets.watch"));
            assert_eq!(content["ticket"]["id"], "WIRE-1");
        }
        other => panic!("expected data, got {other:?}"),
    }
}

#[test]
fn progress_variant_survives_the_crossing() {
    let server = ServerItem::progress(meta(), "scanning".into(), Some(42.5));
    match cross(&server) {
        ClientItem::Progress {
            message,
            percentage,
            metadata,
        } => {
            assert_eq!(message, "scanning");
            // f32 on both sides. plexus-codegen-rust says f64 — that is the
            // drift this assertion exists to notice.
            assert_eq!(percentage, Some(42.5f32));
            assert!(metadata.is_some());
        }
        other => panic!("expected progress, got {other:?}"),
    }
}

#[test]
fn error_variant_survives_the_crossing() {
    let server = ServerItem::error(meta(), "boom".into(), Some("E001".into()), true);
    match cross(&server) {
        ClientItem::Error {
            message,
            code,
            recoverable,
            ..
        } => {
            assert_eq!(message, "boom");
            assert_eq!(code.as_deref(), Some("E001"));
            assert!(recoverable, "the flag must survive, not default");
        }
        other => panic!("expected error, got {other:?}"),
    }
}

#[test]
fn request_variant_survives_the_crossing() {
    let server = ServerItem::request("r-1".into(), json!({"prompt": "continue?"}), 30_000);
    match cross(&server) {
        ClientItem::Request {
            request_id,
            request_data,
            timeout_ms,
        } => {
            assert_eq!(request_id, "r-1");
            assert_eq!(request_data["prompt"], "continue?");
            assert_eq!(timeout_ms, 30_000);
        }
        other => panic!("expected request, got {other:?}"),
    }
}

#[test]
fn done_variant_survives_the_crossing() {
    let server = ServerItem::done(meta());
    match cross(&server) {
        ClientItem::Done { metadata } => assert!(metadata.is_some()),
        other => panic!("expected done, got {other:?}"),
    }
}

/// Every variant's discriminant, side by side. If plexus-core ever renames a
/// tag or adds a variant, this is what says so.
#[test]
fn every_server_variant_tag_is_readable_by_the_client() {
    let cases: Vec<(&str, ServerItem)> = vec![
        ("data", ServerItem::data(meta(), "t".into(), json!({}))),
        ("progress", ServerItem::progress(meta(), "m".into(), None)),
        ("error", ServerItem::error(meta(), "m".into(), None, false)),
        ("request", ServerItem::request("r".into(), json!({}), 0)),
        ("done", ServerItem::done(meta())),
    ];

    for (expected_tag, server) in cases {
        let wire = serde_json::to_value(&server).unwrap();
        assert_eq!(
            wire["type"], expected_tag,
            "plexus-core changed a variant tag: {wire}"
        );
        let client = cross(&server);
        let back = serde_json::to_value(&client).unwrap();
        assert_eq!(
            back["type"], expected_tag,
            "client re-serializes under a different tag: {back}"
        );
    }
}

/// The optional-field difference is *one-directional and deliberate*: the
/// client accepts an item with no metadata (what SSE produces), the server
/// type does not. Pinning it stops someone "fixing" the client type back to
/// required fields and silently breaking the REST transport.
#[test]
fn the_client_type_is_looser_by_design() {
    let sse_shaped = json!({"type": "data", "content": {"ok": true}});

    let client: Result<ClientItem, _> = serde_json::from_value(sse_shaped.clone());
    assert!(client.is_ok(), "client must accept metadata-less items");

    let server: Result<ServerItem, _> = serde_json::from_value(sse_shaped);
    assert!(
        server.is_err(),
        "if plexus-core ever makes metadata optional, this crate's envelope \
         doc-comment needs updating — the split may no longer be needed"
    );
}
