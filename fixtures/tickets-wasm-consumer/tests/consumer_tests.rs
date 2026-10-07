//! MACRO-2 criterion 1, exercised.
//!
//! The wasm32 half of criterion 1 is a build (`cargo build -p
//! tickets-wasm-consumer --target wasm32-unknown-unknown`). This is the
//! behavioural half, run natively over `MemoryTransport` — the one transport
//! that is wasm-safe by construction, so what runs here is what would run in
//! the browser.
//!
//! Note what this crate does **not** depend on: plexus-core, tokio,
//! jsonrpsee, uuid. `cargo tree --target wasm32-unknown-unknown` for this
//! package contains none of them.

use plexus_client::{MemoryTransport, MethodRef, StreamItem, TransportError};
use serde_json::json;
use tickets_api::{Status, Ticket, TicketEvent, TicketsClient};
use tickets_wasm_consumer::{describe, listed_and_watched, mirror_epic, tickets_in_epic};

const LIST: MethodRef = MethodRef::new("tickets", "list");
const WATCH: MethodRef = MethodRef::new("tickets", "watch");

fn ticket(id: &str, parent: Option<&str>) -> Ticket {
    Ticket {
        id: id.into(),
        epic: "MACRO".into(),
        title: format!("ticket {id}"),
        status: Status::Ready,
        parent: parent.map(str::to_string),
        children: vec![],
    }
}

#[tokio::test]
async fn a_consumer_gets_typed_tickets_with_no_wire_code_of_its_own() {
    let client = TicketsClient::new(MemoryTransport::new().script_events(
        LIST,
        &[
            TicketEvent::Ticket { ticket: ticket("MACRO-1", None) },
            TicketEvent::Ticket { ticket: ticket("MACRO-2", Some("MACRO-1")) },
        ],
    ));

    let tickets = tickets_in_epic(&client, "MACRO").await.expect("ok");
    assert_eq!(tickets.len(), 2);
    assert_eq!(tickets[0].id, "MACRO-1");
    // `status` is the server's enum, not a String — menger's frontend
    // flattens it to a String today and keeps a third copy of the vocabulary.
    assert_eq!(tickets[1].status, Status::Ready);
    assert_eq!(tickets[1].parent.as_deref(), Some("MACRO-1"));

    // and the call it made:
    assert_eq!(
        client.transport().params_of(LIST),
        Some(json!({"epic": "MACRO"})),
        "the params object is built from the server's own signature"
    );
}

#[tokio::test]
async fn the_watch_mirror_folds_a_live_stream() {
    let client = TicketsClient::new(MemoryTransport::new().script_events(
        WATCH,
        &[
            TicketEvent::Created { ticket: ticket("MACRO-1", None) },
            TicketEvent::Created { ticket: ticket("MACRO-2", None) },
            TicketEvent::StatusChanged {
                ticket: Ticket {
                    status: Status::Complete,
                    ..ticket("MACRO-1", None)
                },
                previous: Status::Ready,
            },
        ],
    ));

    let mirror = mirror_epic(&client, None).await.expect("ok").expect("no refusal");
    assert_eq!(mirror.len(), 2, "the status change updated in place");
    assert_eq!(mirror[0].status, Status::Complete);
}

#[tokio::test]
async fn an_activation_error_variant_and_a_transport_failure_land_differently() {
    let in_band = TicketsClient::new(MemoryTransport::new().script_events(
        WATCH,
        &[TicketEvent::Error { message: "epic not found".into() }],
    ));
    let refused = mirror_epic(&in_band, None).await.expect("the call itself succeeded");
    assert_eq!(refused, Err("epic not found".to_string()));

    let dead = TicketsClient::new(
        MemoryTransport::new().script_failure(WATCH, TransportError::connect("refused")),
    );
    let err = mirror_epic(&dead, None).await.unwrap_err();
    assert!(err.is_transport(), "{err:?}");
}

/// The client is honest about a server that changed underneath it: a payload
/// the event type cannot read is an error, not a silently-dropped event.
/// menger's frontend today does `serde_json::from_value(..).ok()` and drops.
#[tokio::test]
async fn a_changed_server_payload_surfaces_instead_of_vanishing() {
    let client = TicketsClient::new(MemoryTransport::new().script(
        LIST,
        vec![
            StreamItem::data(json!({
                "type": "ticket",
                "ticket": {"id": "MACRO-1", "epic": "MACRO", "title": "t",
                           "status": "Ready" /* `children` missing */}
            })),
            StreamItem::done(),
        ],
    ));

    let err = tickets_in_epic(&client, "MACRO").await.unwrap_err();
    assert!(!err.is_transport() && !err.is_remote(), "a decode error: {err:?}");
}

/// `list` and `watch` merged into one listener through the derived
/// aggregate.
#[tokio::test]
async fn two_calls_merge_through_the_aggregate() {
    let client = TicketsClient::new(
        MemoryTransport::new()
            .script_events(LIST, &[TicketEvent::Ticket { ticket: ticket("MACRO-1", None) }])
            .script_events(
                WATCH,
                &[
                    TicketEvent::Created { ticket: ticket("MACRO-2", None) },
                    TicketEvent::StatusChanged {
                        ticket: Ticket {
                            status: Status::Complete,
                            ..ticket("MACRO-1", None)
                        },
                        previous: Status::Ready,
                    },
                ],
            ),
    );
    let mut mirror = listed_and_watched(&client, "MACRO").await.expect("ok");
    mirror.sort_by(|a, b| a.id.cmp(&b.id));
    let ids: Vec<&str> = mirror.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(ids, ["MACRO-1", "MACRO-2"]);
}

#[test]
fn the_status_vocabulary_is_the_servers_enum_not_a_copy() {
    assert_eq!(describe(Status::Pending), "awaiting ratification");
    assert_eq!(describe(Status::Complete), "done");
}
