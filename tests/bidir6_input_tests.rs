//! BIDIR-6: `Input<T>` in the macro. The parameter is not a param, the schema
//! carries `input` for it, the dispatch opens it, a method without one is
//! untouched, and the emitted client takes a stream. The wire is
//! plexus-transport's test (`tests/duplex_wire.rs`).

use std::sync::Arc;

use futures::stream::{Stream, StreamExt};
use plexus_core::plexus::{Activation, Input, PlexusError};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Token {
    pub t: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum IngestEvent {
    Ingested { count: u32 },
}

impl plexus_client::FromClientError for IngestEvent {
    fn from_client_error(error: plexus_client::ClientError) -> Self {
        panic!("{error}")
    }
}

#[derive(Clone)]
pub struct IngestActivation;

#[plexus_macros::activation(namespace = "ingest", crate_path = "plexus_core")]
impl IngestActivation {
    /// Take a run's tokens
    #[plexus_macros::method(input_window = 256, input_max_items = 1000)]
    pub async fn tokens(
        &self,
        /// the run they belong to
        run: String,
        /// the tokens, as they arrive
        tokens: Input<Token>,
    ) -> impl Stream<Item = IngestEvent> + Send + 'static {
        let _ = run;
        async_stream::stream! {
            let count = tokens.into_stream().count().await as u32;
            yield IngestEvent::Ingested { count };
        }
    }

    /// The input first: it keeps its place among the arguments
    #[plexus_macros::method]
    pub async fn first(&self, tokens: Input<Token>, run: String) -> impl Stream<Item = IngestEvent> + Send + 'static {
        let _ = (tokens, run);
        futures::stream::empty()
    }

    /// No input
    #[plexus_macros::method]
    pub async fn plain(&self, run: String) -> impl Stream<Item = IngestEvent> + Send + 'static {
        let _ = run;
        futures::stream::empty()
    }
}

fn method(name: &str) -> plexus_core::plexus::MethodSchema {
    IngestActivation.plugin_schema().methods.into_iter().find(|m| m.name == name).unwrap()
}

#[test]
fn the_schema_carries_input_beside_params() {
    let m = method("tokens");
    let input = serde_json::to_value(m.input.as_ref().expect("input")).unwrap();
    assert_eq!(input["properties"]["t"]["type"], "string", "T's schema: {input}");
    assert_eq!(input["description"], "the tokens, as they arrive");
    let params = serde_json::to_value(m.params.as_ref().unwrap()).unwrap();
    let keys: Vec<&String> = params["properties"].as_object().unwrap().keys().collect();
    assert_eq!(keys, ["run"], "the params object is the other parameters");
}

#[test]
fn a_method_without_input_has_none() {
    assert!(method("plain").input.is_none());
    assert!(!serde_json::to_string(&method("plain")).unwrap().contains("\"input\""));
}

#[tokio::test]
async fn with_no_call_under_it_the_method_is_refused_and_named() {
    let refused = IngestActivation.call("tokens", serde_json::json!({"run": "r1"}), None, None).await;
    match refused {
        Err(PlexusError::ExecutionError(m)) => assert!(m.contains("ingest.tokens") && m.contains("WebSocket"), "{m}"),
        other => panic!("{:?}", other.map(|_| "a stream")),
    }
}

#[tokio::test]
async fn the_params_are_read_before_the_input_opens() {
    let refused = IngestActivation.call("tokens", serde_json::json!({}), None, None).await;
    assert!(matches!(refused, Err(PlexusError::InvalidParams(m)) if m.contains("run")));
}

#[tokio::test]
async fn the_trait_takes_a_stream_of_the_items() {
    use futures::stream::BoxStream;
    let tokens: BoxStream<'static, Token> = futures::stream::iter(vec![Token { t: "a".into() }, Token { t: "b".into() }]).boxed();
    let events: Vec<IngestEvent> = Ingest::tokens(&IngestActivation, "r1".into(), tokens).collect().await;
    assert!(matches!(events.as_slice(), [IngestEvent::Ingested { count: 2 }]));
}

#[tokio::test]
async fn a_transport_with_no_input_half_says_so() {
    let client = IngestClient::new(Arc::new(plexus_client::MemoryTransport::new()));
    let tokens = futures::stream::iter(vec![Token { t: "a".into() }]);
    let err = client.tokens_all("r1".into(), tokens).await.unwrap_err();
    assert!(err.to_string().contains("ingest.tokens") && err.to_string().contains("input stream"), "{err}");
}

/// The client's input keeps its place, and borrows are fine: a stream of the
/// items, not a `'static` one.
#[tokio::test]
async fn the_emitted_client_takes_a_stream_in_its_place() {
    let client = IngestClient::new(plexus_client::MemoryTransport::new());
    let held = vec![Token { t: "a".into() }];
    let tokens = futures::stream::iter(held.iter().cloned());
    let events = client.first(tokens, "r1".into());
    let got: Vec<_> = events.collect().await;
    assert_eq!(got.len(), 1, "one refusal, no hang");
    let got = client.plain("r1".into()).next().await;
    assert!(got.is_some());
}
