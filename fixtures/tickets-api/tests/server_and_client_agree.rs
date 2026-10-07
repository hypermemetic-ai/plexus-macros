//! The same activation, reached two ways.
//!
//! The point of a generated client is that it and the server cannot disagree
//! — so this drives the *real* activation through `Activation::call` (what a
//! hub does) and asserts the events are exactly what the typed client
//! decodes from the same items. It is also a preview of MACRO-2.B3's
//! in-process transport, without being one: it stops at comparing payloads.

#![cfg(feature = "server")]

use futures::StreamExt;
use plexus_client::{MemoryTransport, MethodRef, StreamItem};
use plexus_core::plexus::{Activation, PlexusStreamItem};
use serde_json::json;
use tickets_api::{Status, TicketEvent, Tickets, TicketsClient};

const GET: MethodRef = MethodRef::new("tickets", "get");

/// Dispatch through the activation and pull out the raw event JSON, the way
/// a transport would.
async fn dispatch(method: &str, params: serde_json::Value) -> Vec<serde_json::Value> {
    let activation = Tickets::new(tickets_api::Store::seeded());
    let stream = activation
        .call(method, params, None, None)
        .await
        .expect("dispatch");
    stream
        .collect::<Vec<PlexusStreamItem>>()
        .await
        .into_iter()
        .filter_map(|i| match i {
            PlexusStreamItem::Data { content, .. } => Some(content),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn what_the_server_emits_is_what_the_client_decodes() {
    // 1. the server's own output for `tickets.get MACRO-2`
    let server_events = dispatch("get", json!({"id": "MACRO-2"})).await;
    assert_eq!(server_events.len(), 1);
    assert_eq!(server_events[0]["type"], "ticket");
    assert_eq!(server_events[0]["ticket"]["id"], "MACRO-2");

    // 2. the same bytes, handed to the typed client
    let items: Vec<StreamItem> = server_events
        .iter()
        .cloned()
        .map(StreamItem::data)
        .chain(std::iter::once(StreamItem::done()))
        .collect();
    let client = TicketsClient::new(MemoryTransport::new().script(GET, items));

    let decoded = client.get("MACRO-2".to_string()).await.expect("decodes");
    match &decoded {
        TicketEvent::Ticket { ticket } => {
            assert_eq!(ticket.id, "MACRO-2");
            assert_eq!(ticket.status, Status::Ready);
            assert_eq!(ticket.parent.as_deref(), Some("MACRO-1"));
        }
        other => panic!("unexpected decode: {other:?}"),
    }

    // 3. and the params it would have sent match the schema's key names
    assert_eq!(client.transport().params_of(GET), Some(json!({"id": "MACRO-2"})));
}

#[tokio::test]
async fn the_missing_ticket_path_agrees_too() {
    let server_events = dispatch("get", json!({"id": "NOPE-1"})).await;
    assert_eq!(server_events[0]["type"], "not_found");

    let items: Vec<StreamItem> = server_events
        .iter()
        .cloned()
        .map(StreamItem::data)
        .chain(std::iter::once(StreamItem::done()))
        .collect();
    let client = TicketsClient::new(MemoryTransport::new().script(GET, items));

    assert_eq!(
        client.get("NOPE-1".to_string()).await.unwrap(),
        TicketEvent::NotFound { id: "NOPE-1".into() }
    );
}

/// The activation still schemas itself — the client is additive, and the
/// method list the schema advertises is the method list the client exposes.
#[test]
fn the_schema_and_the_client_advertise_the_same_methods() {
    let activation = Tickets::default();
    let schema = activation.plugin_schema();

    let mut from_schema: Vec<String> = schema
        .methods
        .iter()
        .map(|m| m.name.clone())
        .filter(|n| n != "schema") // the macro's own introspection method
        .collect();
    from_schema.sort();

    let mut from_client: Vec<String> = vec![
        TicketsClient::<MemoryTransport>::METHOD_HEALTH,
        TicketsClient::<MemoryTransport>::METHOD_GET,
        TicketsClient::<MemoryTransport>::METHOD_LIST,
        TicketsClient::<MemoryTransport>::METHOD_WATCH,
    ]
    .into_iter()
    .map(|m| m.method().to_string())
    .collect();
    from_client.sort();

    assert_eq!(from_schema, from_client);
}
