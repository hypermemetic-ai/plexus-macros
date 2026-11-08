//! ASK-2: `#[plexus_macros::method(asks)]` — a method that answers a call it
//! would act on with a `needs_input` proposal and acts when called again
//! with the token. The macro marks the schema (`asks: true`), keeps
//! `confirm: Option<ConfirmToken>` an ordinary parameter, runs the method as
//! its path so `ask::propose` / `ask::check` seal for it, and takes every
//! signature form.

use futures::StreamExt;
use plexus_core::plexus::ask::{self, ConfirmToken, NeedsInput, Proposal};
use plexus_core::plexus::{Activation, PlexusStreamItem};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WipeEvent {
    NeedsInput(NeedsInput),
    Wiped { what: String },
}

impl plexus_client::FromClientError for WipeEvent {
    fn from_client_error(error: plexus_client::ClientError) -> Self {
        WipeEvent::Wiped { what: error.to_string() }
    }
}

#[derive(Clone)]
pub struct WipeActivation;

#[plexus_macros::activation]
impl WipeActivation {
    /// Wipes `what`, once the caller has said yes (a stream).
    #[plexus_macros::method(asks)]
    async fn wipe(&self, what: String, confirm: Option<ConfirmToken>) -> impl futures::Stream<Item = WipeEvent> + Send + 'static {
        // The plan is built in the body; the stream reads the method path too.
        let plan = json!({ "what": what });
        let early = ask::method_path();
        async_stream::stream! {
            assert_eq!(early, Some("wipe.wipe"));
            assert_eq!(ask::method_path(), Some("wipe.wipe"), "the stream runs as the method");
            match ask::check(confirm.as_ref(), &plan) {
                Ok(()) => yield WipeEvent::Wiped { what },
                Err(_) => yield WipeEvent::NeedsInput(ask::propose(&plan, Proposal::new(format!("wipe {what}")))),
            }
        }
    }

    /// The one-item form asks too.
    #[plexus_macros::method(asks)]
    async fn wipe_one(&self, what: String, confirm: Option<ConfirmToken>) -> WipeEvent {
        let plan = json!({ "one": what });
        match ask::check(confirm.as_ref(), &plan) {
            Ok(()) => WipeEvent::Wiped { what },
            Err(_) => WipeEvent::NeedsInput(ask::propose(&plan, Proposal::new("wipe one"))),
        }
    }

    /// A method that does not ask is not marked.
    #[plexus_macros::method]
    async fn look(&self) -> WipeEvent {
        WipeEvent::Wiped { what: "nothing".into() }
    }
}

async fn frames(method: &str, params: Value) -> Vec<Value> {
    WipeActivation
        .call(method, params, None, None)
        .await
        .expect("dispatches")
        .map(|item| match item {
            PlexusStreamItem::Data { content_type, content, .. } => {
                assert_eq!(content_type, format!("wipe.{method}"));
                content.into_value()
            }
            PlexusStreamItem::Done { .. } => json!("done"),
            other => panic!("unexpected frame {other:?}"),
        })
        .collect()
        .await
}

#[tokio::test]
async fn a_stream_method_proposes_then_acts_on_its_token() {
    let got = frames("wipe", json!({ "what": "logs" })).await;
    assert_eq!(got.len(), 2, "{got:?}");
    assert_eq!(got[0]["type"], "needs_input");
    assert_eq!(got[0]["summary"], "wipe logs");
    assert_eq!(got[1], "done");
    let token = got[0]["token"].clone();
    let got = frames("wipe", json!({ "what": "logs", "confirm": token })).await;
    assert_eq!(got, vec![json!({ "type": "wiped", "what": "logs" }), json!("done")]);
    // Another plan: the token does not carry over.
    let got = frames("wipe", json!({ "what": "db", "confirm": token })).await;
    assert_eq!(got[0]["type"], "needs_input", "{got:?}");
}

#[tokio::test]
async fn a_one_item_method_proposes_then_acts() {
    let got = frames("wipe_one", json!({ "what": "x" })).await;
    assert_eq!(got[0]["type"], "needs_input", "{got:?}");
    let got = frames("wipe_one", json!({ "what": "x", "confirm": got[0]["token"] })).await;
    assert_eq!(got[0], json!({ "type": "wiped", "what": "x" }));
    // A method's path is its own: wipe_one's token is no token for wipe.
    let p = frames("wipe_one", json!({ "what": "x" })).await;
    let got = frames("wipe", json!({ "what": "x", "confirm": p[0]["token"] })).await;
    assert_eq!(got[0]["type"], "needs_input", "{got:?}");
}

#[test]
fn the_schema_says_asks_and_confirm_is_an_optional_parameter() {
    let schema = WipeActivation.plugin_schema();
    let wipe = schema.methods.iter().find(|m| m.name == "wipe").unwrap();
    assert!(wipe.asks);
    let v = serde_json::to_value(wipe).unwrap();
    assert_eq!(v["asks"], true);
    assert!(v.get("bidirectional").is_none(), "{v}");
    let params = &v["params"];
    assert!(params["properties"]["confirm"].to_string().contains("ConfirmToken"), "{params}");
    assert_eq!(params["$defs"]["ConfirmToken"]["type"], "string", "{params}");
    assert_eq!(params["required"], json!(["what"]), "{params}");
    // The proposal is in the item's schema.
    assert!(v["returns"].to_string().contains("needs_input"), "{}", v["returns"]);
    let look = serde_json::to_value(schema.methods.iter().find(|m| m.name == "look").unwrap()).unwrap();
    assert!(look.get("asks").is_none(), "absent while false: {look}");
    assert!(schema.methods.iter().find(|m| m.name == "wipe_one").unwrap().asks);
}
