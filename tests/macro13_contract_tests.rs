//! MACRO-13 · a bodiless activation is a contract.
//!
//! `impl ContractActivation` with every wire method bodiless and no struct
//! declared: the macro emits `ContractActivation<D: Contract>`, delegating to
//! `D`, `impl<D: Contract> Contract for ContractActivation<D>`, and
//! `Contract::server(self)`. So an implementation is served with
//! `hub.register(Impl::new().server())`, and a remote one — the client — is
//! served here as a proxy with `hub.register(ContractClient::new(t).server())`.

use async_stream::stream;
use futures::{Stream, StreamExt};
use plexus_client::{BoxStream, ClientError, FromClientError};
use plexus_core::plexus::{Activation, InProcessTransport, PlexusStreamItem};
use plexus_core::DynamicHub;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContractEvent {
    Pong { text: String },
    Sum { value: i64 },
    Error { message: String },
}

impl FromClientError for ContractEvent {
    fn from_client_error(error: ClientError) -> Self {
        ContractEvent::Error { message: error.to_string() }
    }
}

/// A contract: ping and add.
#[plexus_macros::activation]
impl ContractActivation {
    /// Answer with the text.
    pub async fn ping(&self, /** what to answer */ text: String) -> impl Stream<Item = ContractEvent> + Send + 'static;

    /// Add two numbers.
    pub async fn add(&self, a: i64, b: i64) -> impl Stream<Item = ContractEvent> + Send + 'static;
}

/// An implementation, with nothing of the wire in it.
struct Impl {
    prefix: String,
}

impl Impl {
    fn new() -> Self {
        Impl { prefix: "local:".into() }
    }
}

impl Contract for Impl {
    fn ping(&self, text: String) -> BoxStream<'static, ContractEvent> {
        let text = format!("{}{text}", self.prefix);
        Box::pin(stream! { yield ContractEvent::Pong { text }; })
    }

    fn add(&self, a: i64, b: i64) -> BoxStream<'static, ContractEvent> {
        Box::pin(stream! { yield ContractEvent::Sum { value: a + b }; })
    }
}

async fn data(hub: &DynamicHub, method: &str, params: Value) -> Vec<Value> {
    let stream = hub.route(method, params, None).await.expect("routed");
    stream
        .filter_map(|item| async move {
            match item {
                PlexusStreamItem::Data { content, .. } => Some(content),
                PlexusStreamItem::Error { message, .. } => panic!("error item: {message}"),
                _ => None,
            }
        })
        .collect()
        .await
}

async fn answers_both(hub: &DynamicHub) {
    assert_eq!(
        data(hub, "contract.ping", json!({"text": "hi"})).await,
        vec![json!({"type": "pong", "text": "local:hi"})]
    );
    assert_eq!(
        data(hub, "contract.add", json!({"a": 2, "b": 40})).await,
        vec![json!({"type": "sum", "value": 42})]
    );
}

#[tokio::test]
async fn an_implementation_is_served_with_server() {
    let activation: ContractActivation<Impl> = Impl::new().server();
    assert_eq!(activation.namespace(), "contract");
    assert_eq!(activation.methods(), vec!["ping", "add", "schema"]);
    assert_eq!(activation.description(), "A contract: ping and add.");
    let hub = DynamicHub::new("toy").register(activation);
    answers_both(&hub).await;
}

#[tokio::test]
async fn a_client_is_served_as_a_proxy_with_server() {
    let remote = InProcessTransport::new(Impl::new().server());
    let proxy = ContractClient::new(remote).server();
    let hub = DynamicHub::new("toy").register(proxy);
    answers_both(&hub).await;
}

#[tokio::test]
async fn the_served_contract_is_itself_a_contract() {
    fn is_contract<T: Contract>(_: &T) {}
    let served = Impl::new().server();
    is_contract(&served);
    let all: Vec<Box<dyn Contract>> = vec![
        Box::new(Impl::new()),
        Box::new(served.clone()),
        Box::new(ContractClient::new(InProcessTransport::new(served))),
    ];
    for c in &all {
        let got: Vec<ContractEvent> = c.add(1, 2).collect().await;
        assert_eq!(got, vec![ContractEvent::Sum { value: 3 }]);
    }
}

#[test]
fn the_contracts_schema_carries_its_doc_comments() {
    let schema = serde_json::to_string(&Impl::new().server().plugin_schema()).unwrap();
    assert!(schema.contains("Answer with the text."), "{schema}");
    assert!(schema.contains("what to answer"), "{schema}");
}
