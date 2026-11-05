//! MACRO-13 · a bodiless activation is a contract.
//!
//! `impl ContractActivation` with every wire method bodiless and no struct
//! declared: the macro emits `ContractActivation<D: Contract>`, delegating to
//! `D`, `impl<D: Contract> Contract for ContractActivation<D>`, and
//! `Contract::server(self)`. So an implementation is served with
//! `hub.register(Impl::new().server())`. RETURNS-47: the client implements
//! no trait, so a remote one is served here as a proxy by an implementation
//! that calls the client and says its failures in band, written by hand
//! (`Proxy`): `hub.register(Proxy(ContractClient::new(t)).server())`.

use async_stream::stream;
use futures::{Stream, StreamExt, TryStreamExt};
use plexus_client::BoxStream;
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
                PlexusStreamItem::Data { content, .. } => Some(content.into_value()),
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

/// A remote implementation, through its client: what the client's own
/// implementation of the trait did before RETURNS-47, written where it is
/// wanted — each call's items as they come, a client failure as the event's
/// own error, and the end.
struct Proxy<T>(ContractClient<T>);

fn in_band<T: plexus_client::Transport + Clone + Send + Sync + 'static>(
    client: ContractClient<T>,
    call: impl FnOnce(&ContractClient<T>) -> futures::stream::BoxStream<'_, Result<ContractEvent, plexus_client::ClientError>> + Send + 'static,
) -> BoxStream<'static, ContractEvent> {
    Box::pin(stream! {
        let mut items = call(&client);
        while let Some(item) = items.next().await {
            match item {
                Ok(event) => yield event,
                Err(e) => {
                    yield ContractEvent::Error { message: e.to_string() };
                    break;
                }
            }
        }
    })
}

impl<T: plexus_client::Transport + Clone + Send + Sync + 'static> Contract for Proxy<T> {
    fn ping(&self, text: String) -> BoxStream<'static, ContractEvent> {
        in_band(self.0.clone(), move |c| c.ping(text).boxed())
    }

    fn add(&self, a: i64, b: i64) -> BoxStream<'static, ContractEvent> {
        in_band(self.0.clone(), move |c| c.add(a, b).boxed())
    }
}

#[tokio::test]
async fn a_client_is_served_as_a_proxy_with_server() {
    let remote = InProcessTransport::new(Impl::new().server());
    let proxy = Proxy(ContractClient::new(remote)).server();
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
        Box::new(Proxy(ContractClient::new(InProcessTransport::new(served)))),
    ];
    for c in &all {
        let got: Vec<ContractEvent> = c.add(1, 2).collect().await;
        assert_eq!(got, vec![ContractEvent::Sum { value: 3 }]);
    }
}

/// A transport that has each item's text and no `Value`s.
#[derive(Clone)]
struct TextOnly;

impl plexus_client::Transport for TextOnly {
    fn call(&self, _: plexus_client::MethodRef, _: Value) -> plexus_client::ItemStream<'_> {
        panic!("a client's contract reads its items through call_raw")
    }

    fn call_raw(&self, _: plexus_client::MethodRef, _: Value) -> plexus_client::RawItemStream<'_> {
        let text = serde_json::value::RawValue::from_string(r#"{"type":"sum","value":5}"#.to_owned()).unwrap();
        Box::pin(futures::stream::iter(vec![
            Ok(plexus_client::RawItem::Data { content_type: Some("contract.add".into()), content: plexus_client::Content::from(text) }),
            Ok(plexus_client::RawItem::Done),
        ]))
    }
}

/// PERF.115-36: a contract's client reads through `call_raw`, the content
/// decoded straight from the wire's text.
#[tokio::test]
async fn a_contracts_client_decodes_the_wires_text_through_call_raw() {
    let got: Vec<ContractEvent> = ContractClient::new(TextOnly).add(2, 3).try_collect().await.unwrap();
    assert_eq!(got, vec![ContractEvent::Sum { value: 5 }]);
}

#[test]
fn the_contracts_schema_carries_its_doc_comments() {
    let schema = serde_json::to_string(&Impl::new().server().plugin_schema()).unwrap();
    assert!(schema.contains("Answer with the text."), "{schema}");
    assert!(schema.contains("what to answer"), "{schema}");
}

// ------------------------------------------------- RETURNS-36: every form

/// A contract whose methods have their own return types (RETURNS-36): a
/// one-item method is a
/// boxed future in the trait, and an implementation served through the
/// contract answers it with one data item.
mod own_types {
    use super::*;
    use plexus_client::BoxFuture;

    #[plexus_macros::activation]
    impl FormsActivation {
        /// One item.
        #[plexus_macros::method]
        pub async fn ping(&self, text: String) -> String;

        /// One item, fallible.
        #[plexus_macros::method]
        pub async fn half(&self, n: i64) -> Result<i64, String>;

        /// Many.
        pub async fn upto(&self, n: i64) -> impl Stream<Item = String> + Send + 'static;
    }

    struct Impl;

    impl Forms for Impl {
        fn ping(&self, text: String) -> BoxFuture<'static, String> {
            Box::pin(async move { format!("pong {text}") })
        }
        fn half(&self, n: i64) -> BoxFuture<'static, Result<i64, String>> {
            Box::pin(async move { if n % 2 == 0 { Ok(n / 2) } else { Err(format!("{n} is odd")) } })
        }
        fn upto(&self, n: i64) -> BoxStream<'static, String> {
            Box::pin(futures::stream::iter((1..=n).map(|i| i.to_string())))
        }
    }

    #[tokio::test]
    async fn each_form_is_served_through_the_contract() {
        let service = DynamicHub::new("svc").register(Impl.server());
        assert_eq!(data(&service, "forms.ping", json!({"text": "x"})).await, vec![json!("pong x")]);
        assert_eq!(data(&service, "forms.half", json!({"n": 4})).await, vec![json!({"ok": 2})]);
        assert_eq!(data(&service, "forms.half", json!({"n": 3})).await, vec![json!({"err": "3 is odd"})]);
        assert_eq!(data(&service, "forms.upto", json!({"n": 2})).await, vec![json!("1"), json!("2")]);
        // … and the client, whose one-item methods answer `Result`s
        // (RETURNS-47: it implements no trait).
        let client = FormsClient::new(InProcessTransport::new(Impl.server()));
        assert_eq!(client.ping("y".into()).await.unwrap(), "pong y");
        assert_eq!(client.half(3).await.unwrap(), Err("3 is odd".to_string()));
    }
}
