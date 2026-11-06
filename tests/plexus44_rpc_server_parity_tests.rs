//! PLEXUS-44: the jsonrpsee server trait `#[activation]` writes answers on
//! the wire exactly as the one jsonrpsee's own `#[rpc(server)]` wrote.
//!
//! `Reference` is the trait plexus-macros 0.8.0 emitted, written out and
//! expanded by jsonrpsee's attribute; its implementation hands each call to
//! the activation's generated `ParityActivationRpcServer`, so the items are
//! the same and only the registration and parameter decoding differ. Every
//! request below goes to both modules; the answers and the notifications
//! must be equal, the subscription ids aside.

use std::time::Duration;

use futures::Stream;
use jsonrpsee::core::SubscriptionResult;
use jsonrpsee::proc_macros::rpc;
use jsonrpsee::{PendingSubscriptionSink, RpcModule};
use plexus_client::{ClientError, FromClientError};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Said {
    pub text: String,
}

impl FromClientError for Said {
    fn from_client_error(e: ClientError) -> Self {
        Said { text: e.to_string() }
    }
}

#[derive(Clone, Copy, Default)]
pub struct ParityActivation;

/// Parity.
#[plexus_macros::activation(namespace = "par", version = "1.0.0", crate_path = "plexus_core")]
impl ParityActivation {
    /// Every parameter shape the decoding knows.
    #[plexus_macros::method]
    pub async fn say(
        &self,
        text: String,
        times: u32,
        suffix: Option<String>,
        snake_case_name: Option<u32>,
    ) -> impl Stream<Item = Said> + Send + 'static {
        let suffix = suffix.unwrap_or_default();
        let extra = snake_case_name.unwrap_or(0);
        futures::stream::iter((0..times + extra).map(move |_| Said { text: format!("{text}{suffix}") }))
    }

    /// No parameters.
    #[plexus_macros::method]
    pub async fn ping(&self) -> Said {
        Said { text: "pong".into() }
    }
}

/// The trait plexus-macros 0.8.0 handed to jsonrpsee's attribute.
#[rpc(server, namespace = "par")]
pub trait Reference {
    #[subscription(name = "say", unsubscribe = "unsubscribe_say", item = serde_json::Value)]
    async fn say(
        &self,
        text: String,
        times: u32,
        suffix: Option<String>,
        snake_case_name: Option<u32>,
    ) -> SubscriptionResult;

    #[subscription(name = "ping", unsubscribe = "unsubscribe_ping", item = serde_json::Value)]
    async fn ping(&self) -> SubscriptionResult;
}

struct Ref;

#[async_trait::async_trait]
impl ReferenceServer for Ref {
    async fn say(
        &self,
        pending: PendingSubscriptionSink,
        text: String,
        times: u32,
        suffix: Option<String>,
        snake_case_name: Option<u32>,
    ) -> SubscriptionResult {
        ParityActivationRpcServer::say(&ParityActivation, pending, text, times, suffix, snake_case_name).await
    }

    async fn ping(&self, pending: PendingSubscriptionSink) -> SubscriptionResult {
        ParityActivationRpcServer::ping(&ParityActivation, pending).await
    }
}

/// A request's answer and its notifications, with the subscription id
/// replaced by `"<id>"`.
async fn run<C: Send + Sync + 'static>(module: &RpcModule<C>, request: &Value) -> (Value, Vec<Value>) {
    let (answer, mut rx) = module.raw_json_request(&request.to_string(), 256).await.expect("a request");
    let mut answer: Value = serde_json::from_str(answer.get()).unwrap();
    let mut items = Vec::new();
    if answer.get("result").is_some() {
        answer["result"] = json!("<id>");
        loop {
            let next = tokio::time::timeout(Duration::from_secs(5), rx.recv()).await.expect("an item in time");
            let Some(raw) = next else { break };
            let mut item: Value = serde_json::from_str(raw.get()).unwrap();
            item["params"]["subscription"] = json!("<id>");
            let done = item["params"]["result"]["type"] == "done";
            items.push(item);
            if done {
                break;
            }
        }
    }
    (answer, items)
}

#[tokio::test]
async fn the_written_server_trait_answers_as_jsonrpsees_did() {
    let ours = ParityActivation.into_rpc();
    let theirs = Ref.into_rpc();

    let mut names_ours: Vec<&str> = ours.method_names().collect();
    let mut names_theirs: Vec<&str> = theirs.method_names().collect();
    names_ours.sort();
    names_theirs.sort();
    assert_eq!(names_ours, names_theirs);

    let params = [
        json!(["hi", 2]),
        json!(["hi", 2, "!"]),
        json!(["hi", 1, null, 1]),
        json!(["hi", 1, "!", 1, "extra"]),
        json!(["hi"]),
        json!([]),
        json!([2, "hi"]),
        json!({"text": "hi", "times": 2}),
        json!({"text": "hi", "times": 1, "suffix": "?", "snake_case_name": 1}),
        json!({"text": "hi", "times": 1, "snakeCaseName": 2}),
        json!({"text": "hi"}),
        json!({"text": "hi", "times": "two"}),
        json!({"text": "hi", "times": 1, "unknown": true}),
        json!({}),
        json!(null),
        json!("hi"),
    ];
    for (n, p) in params.iter().enumerate() {
        let request = json!({"jsonrpc": "2.0", "id": n, "method": "par_say", "params": p});
        assert_eq!(run(&ours, &request).await, run(&theirs, &request).await, "params {p}");
    }
    let without = json!({"jsonrpc": "2.0", "id": 99, "method": "par_say"});
    assert_eq!(run(&ours, &without).await, run(&theirs, &without).await, "no params");

    for p in [json!(null), json!([]), json!({}), json!(["x"])] {
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "par_ping", "params": p});
        assert_eq!(run(&ours, &request).await, run(&theirs, &request).await, "ping {p}");
    }
}
