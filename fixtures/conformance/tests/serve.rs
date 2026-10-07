//! RETURNS-8: the macro's server half reproduces the conformance fixture.
//!
//! `Conformance` is served in-process and what it sends is compared with
//! `expected/`: its schema RPC with `schema.json`, every call of
//! `calls.json` with its `wire/*.jsonl` file or its `errors.jsonl` line, over
//! both dispatch paths — `Activation::call` (what a hub routes to) and the
//! jsonrpsee subscriptions the activation registers itself.
//!
//! Comparison rules (the fixture's README): a `"*"` in a hash field (`hash`,
//! `self_hash`, `plexus_hash`) matches any string; `timestamp` is normalized
//! to 0; key order is not significant (parsed values are compared).

use std::path::PathBuf;
use std::time::Duration;

use conformance::Conformance;
use futures::StreamExt;
use plexus_core::plexus::{Activation, PlexusContext, PlexusStreamItem, RemoteError, StreamMetadata};
use serde::Deserialize;
use serde_json::{json, Map, Value};

const NS: &str = "conformance";

/// Fields whose expected value may be the wildcard `"*"`.
const HASH_FIELDS: [&str; 3] = ["hash", "self_hash", "plexus_hash"];

fn expected_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected")
}

fn read(rel: &str) -> String {
    let path = expected_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The parsed non-empty lines of a `.jsonl` file.
fn jsonl(rel: &str) -> Vec<Value> {
    read(rel)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{rel}: {e}: {l}")))
        .collect()
}

/// One entry of `calls.json`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Call {
    method: String,
    params: Value,
    frames: String,
    #[serde(default)]
    line: Option<usize>,
}

impl Call {
    /// The frames this call must produce.
    fn expected(&self) -> Vec<Value> {
        let frames = jsonl(&self.frames);
        match self.line {
            None => frames,
            Some(n) => vec![frames[n - 1].clone()],
        }
    }

    fn what(&self) -> String {
        format!("{} {}", self.method, self.params)
    }
}

fn calls() -> Vec<Call> {
    serde_json::from_str(&read("calls.json")).expect("calls.json")
}

/// `expected` and `actual` are the same value, with the README's wildcard.
fn assert_matches(expected: &Value, actual: &Value, at: &str) {
    match (expected, actual) {
        (Value::Object(e), Value::Object(a)) => {
            let ek: Vec<&String> = e.keys().collect();
            let ak: Vec<&String> = a.keys().collect();
            let mut eks = ek.clone();
            let mut aks = ak.clone();
            eks.sort();
            aks.sort();
            assert_eq!(eks, aks, "{at}: keys differ\nexpected {expected}\nactual   {actual}");
            for (k, ev) in e {
                let av = &a[k];
                let here = format!("{at}.{k}");
                if HASH_FIELDS.contains(&k.as_str()) && ev == "*" {
                    assert!(av.is_string(), "{here}: a hash is a string, got {av}");
                    continue;
                }
                assert_matches(ev, av, &here);
            }
        }
        (Value::Array(e), Value::Array(a)) => {
            assert_eq!(e.len(), a.len(), "{at}: lengths differ\nexpected {expected}\nactual   {actual}");
            for (i, (ev, av)) in e.iter().zip(a).enumerate() {
                assert_matches(ev, av, &format!("{at}[{i}]"));
            }
        }
        _ => assert_eq!(expected, actual, "{at}"),
    }
}

/// A served frame as JSON, its `timestamp` normalized to 0.
fn normalized(item: &PlexusStreamItem) -> Value {
    let mut v = serde_json::to_value(item).expect("a frame serializes");
    if let Some(ts) = v.get_mut("metadata").and_then(|m| m.get_mut("timestamp")) {
        assert!(ts.is_i64(), "timestamp is an integer: {ts}");
        *ts = json!(0);
    }
    v
}

/// Serve one call the way a hub's `call` route does with the fixture as its
/// own hub: `Activation::call`'s stream when the call dispatches, one error
/// frame (provenance the activation, not recoverable, no `done`) when it does
/// not.
async fn serve(method: &str, params: Value) -> Vec<Value> {
    match Activation::call(&Conformance, method, params, None, None).await {
        Ok(stream) => stream.map(|item| normalized(&item)).collect().await,
        Err(e) => vec![normalized(&PlexusStreamItem::error(
            StreamMetadata::new(vec![NS.into()], PlexusContext::hash()),
            RemoteError::from(&e),
            false,
        ))],
    }
}

/// Acceptance 1: the schema RPC returns `schema.json`.
#[tokio::test]
async fn the_schema_rpc_returns_schema_json() {
    let frames = serve("schema", json!({})).await;
    assert_eq!(frames.len(), 2, "one data frame, then done: {frames:?}");
    assert_eq!(frames[0]["type"], "data");
    assert_eq!(frames[0]["content_type"], "conformance.schema");
    assert_eq!(frames[1]["type"], "done");

    let expected: Value = serde_json::from_str(&read("schema.json")).expect("schema.json");
    assert_matches(&expected, &frames[0]["content"], "schema");

    // The schema the activation states without the RPC is the same one.
    let direct = serde_json::to_value(Conformance.plugin_schema()).unwrap();
    assert_matches(&expected, &direct, "plugin_schema()");
}

/// Acceptance 1: every call of `calls.json` sends exactly its expected
/// frames — the six signature forms (`wire/`) and the three call failures
/// (`errors.jsonl`).
#[tokio::test]
async fn every_call_sends_its_expected_frames() {
    let calls = calls();
    assert_eq!(calls.len(), 10, "calls.json lists 10 calls");
    for call in calls {
        let actual = serve(&call.method, call.params.clone()).await;
        let expected = call.expected();
        assert_eq!(
            actual.len(),
            expected.len(),
            "{}: frame count\nexpected {expected:#?}\nactual   {actual:#?}",
            call.what()
        );
        for (i, (e, a)) in expected.iter().zip(&actual).enumerate() {
            assert_matches(e, a, &format!("{}: frame {i}", call.what()));
        }
    }
}

/// Acceptance 4: the jsonrpsee path — the subscriptions the activation
/// registers — sends the same frames, ending in exactly one `done`.
#[tokio::test]
async fn the_jsonrpsee_path_sends_the_same_frames_and_one_done() {
    let methods = Conformance.into_rpc_methods();
    for call in calls().into_iter().filter(|c| c.line.is_none()) {
        let params: Map<String, Value> = call
            .params
            .as_object()
            .cloned()
            .expect("wire calls take an object");
        let name = format!("{NS}_{}", call.method);
        let mut sub = methods
            .subscribe_unbounded(&name, params)
            .await
            .unwrap_or_else(|e| panic!("{}: subscribe {name}: {e}", call.what()));

        let mut actual = Vec::new();
        loop {
            // The subscription ends when the activation's task drops its
            // sink; a timeout here means it never did.
            let next = tokio::time::timeout(Duration::from_secs(5), sub.next::<Value>())
                .await
                .unwrap_or_else(|_| panic!("{}: subscription never ended", call.what()));
            match next {
                None => break,
                Some(Ok((mut frame, _id))) => {
                    if let Some(ts) = frame.get_mut("metadata").and_then(|m| m.get_mut("timestamp")) {
                        *ts = json!(0);
                    }
                    actual.push(frame);
                }
                Some(Err(e)) => panic!("{}: {e}", call.what()),
            }
        }

        let dones = actual.iter().filter(|f| f["type"] == "done").count();
        assert_eq!(dones, 1, "{}: exactly one done, got {actual:#?}", call.what());
        assert_eq!(actual.last().unwrap()["type"], "done", "{}: done is last", call.what());

        let expected = call.expected();
        assert_eq!(actual.len(), expected.len(), "{}: frame count {actual:#?}", call.what());
        for (i, (e, a)) in expected.iter().zip(&actual).enumerate() {
            assert_matches(e, a, &format!("jsonrpsee {}: frame {i}", call.what()));
        }
    }
}
