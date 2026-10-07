//! RETURNS-8: bidirectional methods dispatch as before, and take the new
//! signature forms too — the channel is injected, the return value goes
//! through its form's wrapper, and `request` frames would merge in as ever.

use futures::StreamExt;
use plexus_core::plexus::{Activation, PlexusStreamItem, StandardBidirChannel};
use serde_json::{json, Value};

pub struct Wizard;

#[plexus_macros::activation]
impl Wizard {
    /// 0.6's shape: a stream of plain items, the channel declared by flag.
    #[plexus_macros::method(bidirectional)]
    async fn ask(&self, ctx: &StandardBidirChannel) -> impl futures::Stream<Item = String> + Send + 'static {
        let _ = ctx;
        futures::stream::iter(vec!["a".to_string(), "b".to_string()])
    }

    /// A one-item Result, the channel inferred from the `ctx` parameter.
    #[plexus_macros::method]
    async fn confirm(&self, ctx: &StandardBidirChannel, question: String) -> Result<String, String> {
        let _ = ctx;
        if question.is_empty() {
            Err("no question".into())
        } else {
            Ok(question)
        }
    }
}

/// `(type, content)` of each frame.
async fn frames(method: &str, params: Value) -> Vec<(String, Value)> {
    Wizard
        .call(method, params, None, None)
        .await
        .expect("dispatches")
        .map(|item| match item {
            PlexusStreamItem::Data { content_type, content, .. } => {
                assert_eq!(content_type, format!("wizard.{method}"));
                ("data".to_string(), content)
            }
            PlexusStreamItem::Done { .. } => ("done".to_string(), Value::Null),
            other => panic!("unexpected frame {other:?}"),
        })
        .collect()
        .await
}

#[tokio::test]
async fn a_bidirectional_stream_method_dispatches_as_before() {
    assert_eq!(
        frames("ask", json!({})).await,
        vec![
            ("data".into(), json!("a")),
            ("data".into(), json!("b")),
            ("done".into(), Value::Null),
        ]
    );
}

#[tokio::test]
async fn a_bidirectional_result_method_sends_ok_or_err() {
    assert_eq!(
        frames("confirm", json!({"question": "sure?"})).await,
        vec![("data".into(), json!({"ok": "sure?"})), ("done".into(), Value::Null)]
    );
    assert_eq!(
        frames("confirm", json!({"question": ""})).await,
        vec![("data".into(), json!({"err": "no question"})), ("done".into(), Value::Null)]
    );
}

#[test]
fn the_schema_says_bidirectional_and_the_form() {
    let schema = Wizard.plugin_schema();
    let confirm = schema.methods.iter().find(|m| m.name == "confirm").unwrap();
    assert!(confirm.bidirectional);
    assert_eq!(serde_json::to_value(confirm.cardinality).unwrap(), "one");
    assert!(confirm.error.is_some());
    let ask = schema.methods.iter().find(|m| m.name == "ask").unwrap();
    assert!(ask.bidirectional);
    assert_eq!(serde_json::to_value(ask.cardinality).unwrap(), "many");
}
