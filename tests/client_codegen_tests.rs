//! The generated client, driven end to end over `MemoryTransport` (MACRO-2;
//! 0.7 shapes RETURNS-9).
//!
//! The fixture activation below is shaped like menger's `tickets` would be on
//! 0.7: one method per signature form, a mix of required and `Option`
//! parameters, and a no-parameter method. `fixtures/conformance` pins the
//! forms against the shared expected JSON over the in-process transport;
//! this file pins what the emitted code does with each wire shape a
//! transport can deliver.

use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use plexus_client::{
    wire_result, ClientError, Frame, MemoryTransport, MethodRef, MethodStream, NoProgress,
    Progress, RemoteError, StreamItem, Transport, TransportError,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

// ---------------------------------------------------------------- fixture

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Health {
    pub version: String,
    pub ticket_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Ticket {
    pub id: String,
    pub title: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum GetError {
    NotFound { id: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Change {
    Created { ticket: Ticket },
    StatusChanged { id: String, status: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WatchError {
    Lagged { missed: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Summary {
    pub imported: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Step {
    Chunk { done: u32, of: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ImportError {
    Unreadable { chunk: u32 },
}

#[derive(Clone, Default)]
pub struct Tickets;

/// The activation. `client` is the only thing that adds the client; the
/// namespace is implicit (`tickets`).
#[plexus_macros::activation(description = "Plans as data", client)]
impl Tickets {
    /// Heartbeat — version and ticket count.
    #[plexus_macros::method]
    pub async fn health(&self) -> Health {
        Health {
            version: "0.7.0".into(),
            ticket_count: 0,
        }
    }

    /// One ticket by id.
    #[plexus_macros::method]
    pub async fn get(&self, id: String) -> Result<Ticket, GetError> {
        Err(GetError::NotFound { id })
    }

    /// Tickets, optionally filtered.
    #[plexus_macros::method]
    pub async fn list(
        &self,
        epic: Option<String>,
        parent: Option<String>,
    ) -> impl futures::Stream<Item = Ticket> + Send + 'static {
        let _ = (epic, parent);
        futures::stream::empty()
    }

    /// Subscribe to changes.
    #[plexus_macros::method]
    pub async fn watch(
        &self,
        epic: Option<String>,
    ) -> impl futures::Stream<Item = Result<Change, WatchError>> + Send + 'static {
        let _ = epic;
        futures::stream::empty()
    }

    /// Import tickets, reporting each chunk.
    #[plexus_macros::method]
    pub async fn import(
        &self,
        source: String,
    ) -> impl futures::Stream<Item = Result<Progress<Summary, Step>, ImportError>> + Send + 'static
    {
        let _ = source;
        futures::stream::empty()
    }
}

const HEALTH: MethodRef = MethodRef::new("tickets", "health");
const GET: MethodRef = MethodRef::new("tickets", "get");
const LIST: MethodRef = MethodRef::new("tickets", "list");
const WATCH: MethodRef = MethodRef::new("tickets", "watch");
const IMPORT: MethodRef = MethodRef::new("tickets", "import");

fn ticket(id: &str) -> Ticket {
    Ticket {
        id: id.into(),
        title: "t".into(),
        parent: None,
    }
}

fn health() -> Health {
    Health {
        version: "0.7.0".into(),
        ticket_count: 7,
    }
}

fn data(method: MethodRef, content: Value) -> StreamItem {
    StreamItem::Data {
        metadata: None,
        content_type: Some(method.qualified()),
        content,
    }
}

/// A Result item as the server puts it on the wire.
fn result_item<T: Serialize, E: Serialize>(method: MethodRef, item: Result<T, E>) -> StreamItem {
    data(method, wire_result::to_value(&item).unwrap())
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
    assert_eq!(C::METHOD_IMPORT.qualified(), "tickets.import");
    // namespace and method stay separable for the transports that need them apart
    assert_eq!(C::METHOD_WATCH.namespace(), "tickets");
    assert_eq!(C::METHOD_WATCH.method(), "watch");
}

/// Each call's type is its method's contract, nothing wider.
#[allow(dead_code, clippy::let_underscore_future)]
fn every_call_is_typed_precisely(c: &TicketsClient<MemoryTransport>) {
    use std::future::Future;
    use std::pin::Pin;
    type One<'a, I> = Pin<Box<dyn Future<Output = Result<I, ClientError>> + Send + 'a>>;
    let _: One<'_, Health> = Box::pin(c.health());
    let _: One<'_, Result<Ticket, GetError>> = Box::pin(c.get("A-1".into()));
    let _: MethodStream<'_, Ticket, NoProgress, TicketsEvent> = c.list(None, None);
    let _: MethodStream<'_, Result<Change, WatchError>, NoProgress, TicketsEvent> = c.watch(None);
    let _: MethodStream<'_, Result<Summary, ImportError>, Step, TicketsEvent> = c.import("x".into());
}

// ---------------------------------------------------------- one-item methods

#[tokio::test]
async fn a_one_item_method_is_an_async_fn_returning_its_item() {
    let client = TicketsClient::new(MemoryTransport::new().script_events(HEALTH, &[health()]));
    assert_eq!(client.health().await, Ok(health()));
}

/// RETURNS-1 decision 8: `?` is the call failing; the activation's no is a
/// value inside.
#[tokio::test]
async fn a_fallible_one_item_method_nests_the_activations_answer() {
    let found = TicketsClient::new(
        MemoryTransport::new().script(GET, vec![result_item::<_, GetError>(GET, Ok(ticket("A-1"))), StreamItem::done()]),
    );
    assert_eq!(found.get("A-1".into()).await, Ok(Ok(ticket("A-1"))));

    let missing = TicketsClient::new(MemoryTransport::new().script(
        GET,
        vec![
            result_item::<Ticket, _>(GET, Err(GetError::NotFound { id: "A-9".into() })),
            StreamItem::done(),
        ],
    ));
    assert_eq!(
        missing.get("A-9".into()).await,
        Ok(Err(GetError::NotFound { id: "A-9".into() }))
    );
}

// --------------------------------------------------------- many-item methods

#[tokio::test]
async fn a_many_item_method_is_a_try_stream_of_its_items() {
    let client = TicketsClient::new(
        MemoryTransport::new().script_events(LIST, &[ticket("A-1"), ticket("A-2")]),
    );
    let tickets: Vec<Ticket> = client.list(None, None).try_collect().await.expect("ok");
    assert_eq!(tickets, vec![ticket("A-1"), ticket("A-2")]);
}

#[tokio::test]
async fn an_err_item_is_a_value_and_the_stream_carries_on() {
    let client = TicketsClient::new(MemoryTransport::new().script(
        WATCH,
        vec![
            result_item::<_, WatchError>(WATCH, Ok(Change::Created { ticket: ticket("A-1") })),
            result_item::<Change, _>(WATCH, Err(WatchError::Lagged { missed: 3 })),
            result_item::<_, WatchError>(
                WATCH,
                Ok(Change::StatusChanged {
                    id: "A-1".into(),
                    status: "Complete".into(),
                }),
            ),
            StreamItem::done(),
        ],
    ));

    let mut seen = Vec::new();
    let mut stream = client.watch(None);
    while let Some(item) = stream.next().await {
        match item.expect("the call did not fail") {
            Ok(change) => seen.push(format!("{change:?}")),
            Err(WatchError::Lagged { missed }) => seen.push(format!("lagged {missed}")),
        }
    }
    assert_eq!(seen.len(), 3);
    assert_eq!(seen[1], "lagged 3");
}

#[tokio::test]
async fn progress_is_dropped_unless_the_call_opts_in() {
    let script = || {
        vec![
            StreamItem::Progress {
                metadata: None,
                content_type: Some(IMPORT.qualified()),
                content: json!({"type": "chunk", "done": 1, "of": 2}),
            },
            result_item::<_, ImportError>(IMPORT, Ok(Summary { imported: 2 })),
            StreamItem::done(),
        ]
    };

    let plain = TicketsClient::new(MemoryTransport::new().script(IMPORT, script()));
    let items: Vec<_> = plain.import("x".into()).try_collect().await.unwrap();
    assert_eq!(items, vec![Ok(Summary { imported: 2 })]);

    let opted = TicketsClient::new(MemoryTransport::new().script(IMPORT, script()));
    let frames: Vec<_> = opted.import("x".into()).with_progress().try_collect().await.unwrap();
    assert_eq!(
        frames,
        vec![
            Frame::Progress(Step::Chunk { done: 1, of: 2 }),
            Frame::Item(Ok(Summary { imported: 2 })),
        ]
    );
}

// ------------------------------------------------------------- the aggregate

#[tokio::test]
async fn events_yields_the_derived_aggregate() {
    let client = TicketsClient::new(MemoryTransport::new().script_events(LIST, &[ticket("A-1")]));
    let events: Vec<TicketsEvent> = client.list(None, None).events().try_collect().await.unwrap();
    assert!(
        matches!(&events[..], [TicketsEvent::List(t)] if t.id == "A-1"),
        "{events:?}"
    );
}

/// RETURNS-1 decision 6: one listener over several calls is ordinary stream
/// plumbing, because every `.events()` has the same type.
#[tokio::test]
async fn several_calls_merge_into_one_listener() {
    let client = TicketsClient::new(
        MemoryTransport::new()
            .script_events(LIST, &[ticket("A-1")])
            .script(
                WATCH,
                vec![
                    result_item::<_, WatchError>(WATCH, Ok(Change::Created { ticket: ticket("A-2") })),
                    StreamItem::done(),
                ],
            ),
    );
    let mut all = futures::stream::select_all([client.watch(None).events(), client.list(None, None).events()]);
    let mut methods = Vec::new();
    while let Some(ev) = all.next().await {
        methods.push(match ev.expect("no call failed") {
            TicketsEvent::List(_) => "list",
            TicketsEvent::Watch(Ok(_)) => "watch",
            other => panic!("unexpected {other:?}"),
        });
    }
    methods.sort();
    assert_eq!(methods, vec!["list", "watch"]);
}

/// The aggregate's wire shape: `{"method": <wire name>, "event": <item>}`, a
/// Result item as `{ok}` / `{err}`; it round-trips.
#[test]
fn the_aggregate_is_keyed_by_method_and_carries_the_item() {
    let cases = [
        (TicketsEvent::Health(health()), json!({"method": "health", "event": {"version": "0.7.0", "ticket_count": 7}})),
        (
            TicketsEvent::Get(Err(GetError::NotFound { id: "A-9".into() })),
            json!({"method": "get", "event": {"err": {"type": "not_found", "id": "A-9"}}}),
        ),
        (TicketsEvent::List(ticket("A-1")), json!({"method": "list", "event": {"id": "A-1", "title": "t"}})),
        (
            TicketsEvent::Import(Ok(Summary { imported: 2 })),
            json!({"method": "import", "event": {"ok": {"imported": 2}}}),
        ),
    ];
    for (event, wire) in cases {
        assert_eq!(serde_json::to_value(&event).unwrap(), wire);
        let back: TicketsEvent = serde_json::from_value(wire.clone()).unwrap();
        assert_eq!(serde_json::to_value(&back).unwrap(), wire);
    }
}

// ------------------------------------------------------------ the params object

#[tokio::test]
async fn a_required_param_is_sent_by_name() {
    let client = TicketsClient::new(MemoryTransport::new().script(
        GET,
        vec![result_item::<_, GetError>(GET, Ok(ticket("A-9"))), StreamItem::done()],
    ));
    let _ = client.get("A-9".to_string()).await;
    assert_eq!(
        client.transport().params_of(GET),
        Some(json!({"id": "A-9"})),
        "the key is the Rust parameter name, which is what the schema declares"
    );
}

/// The rule menger's `Ticket` serialization already follows: absent, not null.
#[tokio::test]
async fn none_options_are_omitted_and_some_options_are_present() {
    async fn sent(epic: Option<&str>, parent: Option<&str>) -> Option<Value> {
        let client = TicketsClient::new(MemoryTransport::new().script_events::<Ticket>(LIST, &[]));
        let _: Vec<_> = client
            .list(epic.map(Into::into), parent.map(Into::into))
            .collect()
            .await;
        client.transport().params_of(LIST)
    }
    assert_eq!(
        sent(None, None).await,
        Some(json!({})),
        "no `\"epic\": null` — the server never sends that shape either"
    );
    assert_eq!(sent(Some("MACRO"), None).await, Some(json!({"epic": "MACRO"})));
    assert_eq!(
        sent(Some("MACRO"), Some("MACRO-2")).await,
        Some(json!({"epic": "MACRO", "parent": "MACRO-2"}))
    );
}

#[tokio::test]
async fn a_method_with_no_params_sends_an_empty_object() {
    let client = TicketsClient::new(MemoryTransport::new().script_events(HEALTH, &[health()]));
    let _ = client.health().await;
    assert_eq!(client.transport().params_of(HEALTH), Some(json!({})));
}

// ------------------------------------------------------------- the error layers

#[tokio::test]
async fn a_call_failure_is_typed_and_a_transport_failure_stays_apart() {
    let remote = TicketsClient::new(MemoryTransport::new().script(
        HEALTH,
        vec![StreamItem::Error {
            metadata: None,
            recoverable: false,
            error: RemoteError::Unauthenticated,
        }],
    ));
    let err = remote.health().await.unwrap_err();
    assert_eq!(err, ClientError::Remote(RemoteError::Unauthenticated));
    assert!(err.is_remote() && !err.is_transport());

    let dead = TicketsClient::new(
        MemoryTransport::new().script_failure(LIST, TransportError::connect("refused")),
    );
    let got: Vec<_> = dead.list(None, None).collect().await;
    assert!(
        matches!(&got[..], [Err(e)] if e.is_transport() && !e.is_remote()),
        "{got:?}"
    );
}

#[tokio::test]
async fn a_payload_the_item_type_cannot_read_is_a_decode_error() {
    let client = TicketsClient::new(MemoryTransport::new().script(
        HEALTH,
        vec![
            // `ticket_count` missing — the server-side shape changed.
            data(HEALTH, json!({"version": "0.7.0"})),
            StreamItem::done(),
        ],
    ));
    let err = client.health().await.unwrap_err();
    assert!(matches!(err, ClientError::Decode { .. }), "{err:?}");
}

#[tokio::test]
async fn a_one_item_method_that_sends_no_item_is_a_contract_break() {
    let client = TicketsClient::new(MemoryTransport::new().script_events::<Health>(HEALTH, &[]));
    match client.health().await {
        Err(ClientError::Decode { content_type, .. }) => {
            assert_eq!(content_type.as_deref(), Some("tickets.health"))
        }
        other => panic!("{other:?}"),
    }
}

// ------------------------------------------------------- transport interchange

/// One transport, several clients — the shape a frontend needs when it talks
/// to more than one activation over a single connection.
#[tokio::test]
async fn a_transport_can_be_shared_by_reference_counted_handle() {
    let shared = Arc::new(MemoryTransport::new().script_events(HEALTH, &[health()]));
    let a = TicketsClient::new(Arc::clone(&shared));
    let b = TicketsClient::new(Arc::clone(&shared));
    assert!(a.health().await.is_ok());
    assert!(b.health().await.is_ok());
    assert_eq!(shared.calls().len(), 2);
}

/// The generated client names no transport, so it is usable with a
/// `Box<dyn Transport>` a consumer picks at runtime.
#[tokio::test]
async fn the_client_is_generic_over_the_transport() {
    let boxed: Box<dyn Transport + Send + Sync> =
        Box::new(MemoryTransport::new().script_events(HEALTH, &[health()]));
    let client = TicketsClient::new(boxed);
    assert_eq!(client.health().await, Ok(health()));
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
    // not a `#[method]`, so it is deliberately not on the client. A consumer
    // that wants it reads `Transport::call` directly.
    assert_eq!(methods, vec!["get", "health", "import", "list", "schema", "watch"]);

    let stream = a.call("health", json!({}), None, None).await.expect("dispatch");
    let items: Vec<_> = stream.collect().await;
    assert!(
        items
            .iter()
            .any(|i| matches!(i, plexus_core::plexus::PlexusStreamItem::Data { .. })),
        "the activation still dispatches: {items:?}"
    );
}
