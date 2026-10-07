//! MACRO-2 · the generated client, driven end to end.
//!
//! The fixture activation below is shaped like menger's `tickets`: an event
//! enum tagged `#[serde(tag = "type", rename_all = "snake_case")]`, a mix of
//! required and `Option` parameters, a no-parameter method, and one
//! subscription method. What is asserted here is what MFE-8 will rely on.

use futures::StreamExt;
use plexus_client::{
    ClientError, ClientEvent, MemoryTransport, MethodRef, StreamItem, Transport, TransportError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;

// ---------------------------------------------------------------- fixture

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum TicketsEvent {
    Health { version: String, ticket_count: u64 },
    Ticket { ticket: Ticket },
    NotFound { id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Clone, Default)]
pub struct Tickets;

/// The activation. `client` is the only thing this build adds to it.
#[plexus_macros::activation(
    namespace = "tickets",
    description = "Plans as data",
    client
)]
impl Tickets {
    /// Heartbeat — version and ticket count.
    #[plexus_macros::method]
    pub async fn health(&self) -> impl futures::Stream<Item = TicketsEvent> + Send + 'static {
        futures::stream::iter(vec![TicketsEvent::Health {
            version: "0.6.0".into(),
            ticket_count: 0,
        }])
    }

    /// One ticket by id.
    #[plexus_macros::method]
    pub async fn get(
        &self,
        id: String,
    ) -> impl futures::Stream<Item = TicketsEvent> + Send + 'static {
        futures::stream::iter(vec![TicketsEvent::NotFound { id }])
    }

    /// Tickets, optionally filtered.
    #[plexus_macros::method]
    pub async fn list(
        &self,
        epic: Option<String>,
        parent: Option<String>,
    ) -> impl futures::Stream<Item = TicketsEvent> + Send + 'static {
        let _ = (epic, parent);
        futures::stream::empty()
    }

    /// Subscribe to changes.
    #[plexus_macros::method]
    pub async fn watch(
        &self,
        epic: Option<String>,
    ) -> impl futures::Stream<Item = TicketsEvent> + Send + 'static {
        let _ = epic;
        futures::stream::empty()
    }
}

fn ticket(id: &str) -> Ticket {
    Ticket {
        id: id.into(),
        title: "t".into(),
        parent: None,
    }
}

// ------------------------------------------------------- the client exists

#[test]
fn the_client_is_generated_with_the_activations_namespace() {
    assert_eq!(TicketsClient::<MemoryTransport>::NAMESPACE, "tickets");
}

#[test]
fn every_method_gets_a_const_method_ref_qualified_by_the_namespace() {
    type C = TicketsClient<MemoryTransport>;
    assert_eq!(C::METHOD_HEALTH.qualified(), "tickets.health");
    assert_eq!(C::METHOD_GET.qualified(), "tickets.get");
    assert_eq!(C::METHOD_LIST.qualified(), "tickets.list");
    assert_eq!(C::METHOD_WATCH.qualified(), "tickets.watch");
    // namespace and method stay separable for the transports that need them apart
    assert_eq!(C::METHOD_WATCH.namespace(), "tickets");
    assert_eq!(C::METHOD_WATCH.method(), "watch");
}

// ----------------------------------------------------- typed events, no strings

#[tokio::test]
async fn a_call_returns_the_activations_own_event_type() {
    const HEALTH: MethodRef = MethodRef::new("tickets", "health");
    let client = TicketsClient::new(MemoryTransport::new().script_events(
        HEALTH,
        &[TicketsEvent::Health {
            version: "0.6.0".into(),
            ticket_count: 7,
        }],
    ));

    let events = client.health_all().await.expect("ok");
    assert_eq!(
        events,
        vec![TicketsEvent::Health {
            version: "0.6.0".into(),
            ticket_count: 7
        }]
    );
}

#[tokio::test]
async fn the_streaming_form_yields_events_as_they_arrive() {
    const WATCH: MethodRef = MethodRef::new("tickets", "watch");
    let client = TicketsClient::new(MemoryTransport::new().script_events(
        WATCH,
        &[
            TicketsEvent::Ticket { ticket: ticket("A-1") },
            TicketsEvent::Ticket { ticket: ticket("A-2") },
        ],
    ));

    let mut seen = Vec::new();
    let mut stream = client.watch(None);
    while let Some(ev) = stream.next().await {
        if let ClientEvent::Data(TicketsEvent::Ticket { ticket }) = ev.expect("ok") {
            seen.push(ticket.id);
        }
    }
    assert_eq!(seen, vec!["A-1".to_string(), "A-2".to_string()]);
}

// ------------------------------------------------------------ the params object

#[tokio::test]
async fn a_required_param_is_sent_by_name() {
    const GET: MethodRef = MethodRef::new("tickets", "get");
    let client = TicketsClient::new(
        MemoryTransport::new().script_events(GET, &[TicketsEvent::NotFound { id: "A-9".into() }]),
    );

    let _ = client.get_all("A-9".to_string()).await;
    assert_eq!(
        client.transport().params_of(GET),
        Some(json!({"id": "A-9"})),
        "the key is the Rust parameter name, which is what the schema declares"
    );
}

/// The rule menger's `Ticket` serialization already follows: absent, not null.
#[tokio::test]
async fn none_options_are_omitted_and_some_options_are_present() {
    const LIST: MethodRef = MethodRef::new("tickets", "list");

    let all_none = TicketsClient::new(MemoryTransport::new().script_events::<TicketsEvent>(LIST, &[]));
    let _ = all_none.list_all(None, None).await;
    assert_eq!(
        all_none.transport().params_of(LIST),
        Some(json!({})),
        "no `\"epic\": null` — the server never sends that shape either"
    );

    let one_set = TicketsClient::new(MemoryTransport::new().script_events::<TicketsEvent>(LIST, &[]));
    let _ = one_set.list_all(Some("MACRO".into()), None).await;
    assert_eq!(
        one_set.transport().params_of(LIST),
        Some(json!({"epic": "MACRO"}))
    );

    let both = TicketsClient::new(MemoryTransport::new().script_events::<TicketsEvent>(LIST, &[]));
    let _ = both.list_all(Some("MACRO".into()), Some("MACRO-2".into())).await;
    assert_eq!(
        both.transport().params_of(LIST),
        Some(json!({"epic": "MACRO", "parent": "MACRO-2"}))
    );
}

#[tokio::test]
async fn a_method_with_no_params_sends_an_empty_object() {
    const HEALTH: MethodRef = MethodRef::new("tickets", "health");
    let client = TicketsClient::new(MemoryTransport::new().script_events::<TicketsEvent>(HEALTH, &[]));
    let _ = client.health_all().await;
    assert_eq!(client.transport().params_of(HEALTH), Some(json!({})));
}

// ------------------------------------------------------------- the error layers

#[tokio::test]
async fn an_in_band_error_and_a_transport_failure_stay_distinguishable() {
    const HEALTH: MethodRef = MethodRef::new("tickets", "health");

    let remote = TicketsClient::new(MemoryTransport::new().script(
        HEALTH,
        vec![StreamItem::Error {
            metadata: None,
            recoverable: false,
            error: plexus_client::RemoteError::Unauthenticated,
        }],
    ));
    let err = remote.health_all().await.unwrap_err();
    assert!(err.is_remote() && !err.is_transport(), "{err:?}");

    let dead = TicketsClient::new(
        MemoryTransport::new().script_failure(HEALTH, TransportError::connect("refused")),
    );
    let err = dead.health_all().await.unwrap_err();
    assert!(err.is_transport() && !err.is_remote(), "{err:?}");
}

#[tokio::test]
async fn a_payload_the_event_type_cannot_read_is_a_decode_error() {
    const HEALTH: MethodRef = MethodRef::new("tickets", "health");
    let client = TicketsClient::new(MemoryTransport::new().script(
        HEALTH,
        vec![
            // `ticket_count` missing — the server-side shape changed.
            StreamItem::data(json!({"type": "health", "version": "0.6.0"})),
            StreamItem::done(),
        ],
    ));
    let err = client.health_all().await.unwrap_err();
    assert!(matches!(err, ClientError::Decode { .. }), "{err:?}");
}

// ------------------------------------------------------- transport interchange

/// One transport, several clients — the shape MFE-8 needs when the frontend
/// talks to more than one activation over a single connection.
#[tokio::test]
async fn a_transport_can_be_shared_by_reference_counted_handle() {
    const HEALTH: MethodRef = MethodRef::new("tickets", "health");
    let shared = Arc::new(MemoryTransport::new().script_events(
        HEALTH,
        &[TicketsEvent::Health {
            version: "0.6.0".into(),
            ticket_count: 1,
        }],
    ));

    let a = TicketsClient::new(Arc::clone(&shared));
    let b = TicketsClient::new(Arc::clone(&shared));
    assert!(a.health_all().await.is_ok());
    assert!(b.health_all().await.is_ok());
    assert_eq!(shared.calls().len(), 2);
}

/// The generated client names no transport, so it is usable with a
/// `Box<dyn Transport>` a consumer picks at runtime.
#[tokio::test]
async fn the_client_is_generic_over_the_transport() {
    const HEALTH: MethodRef = MethodRef::new("tickets", "health");
    let boxed: Box<dyn Transport + Send + Sync> = Box::new(MemoryTransport::new().script_events(
        HEALTH,
        &[TicketsEvent::Health {
            version: "0.6.0".into(),
            ticket_count: 2,
        }],
    ));
    let client = TicketsClient::new(boxed);
    assert_eq!(client.health_all().await.unwrap().len(), 1);
}

// ------------------------------------------------------------ the server half

/// The activation still has its server side. The client is additive.
#[tokio::test]
async fn the_server_side_expansion_is_untouched() {
    use plexus_core::plexus::Activation;
    let a = Tickets;
    assert_eq!(a.namespace(), "tickets");
    let mut methods = a.methods();
    methods.sort();
    // `schema` is the macro's own synthesized introspection method — it is
    // not a `#[method]`, so it has no event type and is deliberately not on
    // the client. A consumer that wants it reads `Transport::call` directly.
    assert_eq!(methods, vec!["get", "health", "list", "schema", "watch"]);

    let stream = a.call("health", json!({}), None, None).await.expect("dispatch");
    let items: Vec<_> = stream.collect().await;
    assert!(
        items
            .iter()
            .any(|i| matches!(i, plexus_core::plexus::PlexusStreamItem::Data { .. })),
        "the activation still dispatches: {items:?}"
    );
}
