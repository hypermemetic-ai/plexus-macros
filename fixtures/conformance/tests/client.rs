//! RETURNS-9: the macro's client half reproduces the conformance fixture.
//!
//! The emitted `ConformanceClient` drives the activation over plexus-core's
//! `InProcessTransport` — no server, the same frames `tests/serve.rs` pins —
//! and what it decodes is compared with `expected/`:
//!
//! - every call of `calls.json` sends exactly its stated params, and decodes
//!   to the items its `wire/*.jsonl` file encodes (`load` and `warm` opted in
//!   to progress yield their frames in order; `scan` carries on past its
//!   err);
//! - every `errors.jsonl` call surfaces as `ClientError::Remote` carrying
//!   that line's `RemoteError`;
//! - the derived `ConformanceEvent` serializes the items to
//!   `aggregate.jsonl` byte for byte, and deserializes back.
//!
//! The expected items are read from the wire files with the same decoders
//! the client uses (`serde_json::from_value` for a plain item,
//! `wire_result::from_value` for a Result item), so a test here fails on
//! the client, not on a second copy of the contract.

use std::collections::BTreeMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Mutex;

use conformance::{
    Conformance, ConformanceClient, ConformanceEvent, LoadError, LoadStep, Loaded, LookupError, Pong,
    Row, ScanError, Thing, Tick, Warmed,
};
use futures::{StreamExt, TryStreamExt};
use plexus_client::{
    call_one, decode, wire_result, ClientError, Decoder, Frame, ItemStream, MemoryTransport,
    MethodRef, MethodStream, NoProgress, Note, RemoteError, StreamItem, Transport,
};
use plexus_core::plexus::InProcessTransport;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde_json::{json, Value};

type Client = ConformanceClient<Recording>;

// ------------------------------------------------------------------ fixture files

fn expected_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected")
}

fn read(rel: &str) -> String {
    let path = expected_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// The frames of a `.jsonl` file, as the client-side envelope.
fn frames(rel: &str) -> Vec<StreamItem> {
    read(rel)
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("{rel}: {e}: {l}")))
        .collect()
}

/// What a wire file encodes, in order: its data frames decoded as `I`, its
/// progress frames as `P`.
fn expected_frames<I, P: DeserializeOwned>(rel: &str, item: Decoder<I>) -> Vec<Frame<I, P>> {
    frames(rel)
        .into_iter()
        .filter_map(|f| match f {
            StreamItem::Data { content, .. } => {
                Some(Frame::Item(item(content).unwrap_or_else(|e| panic!("{rel}: {e}"))))
            }
            StreamItem::Progress { content, .. } => Some(Frame::Progress(
                serde_json::from_value(content).unwrap_or_else(|e| panic!("{rel}: {e}")),
            )),
            _ => None,
        })
        .collect()
}

/// A wire file's items alone.
fn expected_items<I>(rel: &str, item: Decoder<I>) -> Vec<I> {
    expected_frames::<I, Value>(rel, item)
        .into_iter()
        .filter_map(Frame::into_item)
        .collect()
}

/// One entry of `calls.json`.
#[derive(Debug, Deserialize)]
struct Call {
    method: String,
    params: Value,
    frames: String,
    #[serde(default)]
    line: Option<usize>,
}

fn calls() -> Vec<Call> {
    serde_json::from_str(&read("calls.json")).expect("calls.json")
}

/// The params `calls.json` states for a wire file.
fn stated_params(frames_file: &str) -> Value {
    calls()
        .into_iter()
        .find(|c| c.frames == frames_file)
        .unwrap_or_else(|| panic!("calls.json has no call for {frames_file}"))
        .params
}

// ------------------------------------------------------------------ the transport

/// `InProcessTransport` over the fixture, recording each call's params so
/// the test can hold them against `calls.json`.
struct Recording {
    inner: InProcessTransport<Conformance>,
    calls: Mutex<Vec<(String, Value)>>,
}

impl Recording {
    fn new() -> Self {
        Self {
            inner: InProcessTransport::new(Conformance),
            calls: Mutex::new(Vec::new()),
        }
    }

    /// The params of the last call.
    fn last_params(&self) -> Value {
        self.calls.lock().unwrap().last().expect("a call").1.clone()
    }
}

impl Transport for Recording {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        self.calls
            .lock()
            .unwrap()
            .push((method.qualified(), params.clone()));
        self.inner.call(method, params)
    }
}

fn client() -> Client {
    ConformanceClient::new(Recording::new())
}

// ------------------------------------------------------------------ precise types

/// Every call's result is typed precisely: one-item methods are `async fn`s
/// returning their item, many-item methods `MethodStream<'_, I, P,
/// ConformanceEvent>` with `P` the declared progress (`Note` by default,
/// `NoProgress` when none). This compiles or the test does not.
#[allow(dead_code, clippy::let_underscore_future)]
fn the_client_is_typed_precisely(c: &ConformanceClient<MemoryTransport>) {
    type One<'a, I> = Pin<Box<dyn Future<Output = Result<I, ClientError>> + Send + 'a>>;
    let _: One<'_, Pong> = Box::pin(c.ping());
    let _: One<'_, Result<Thing, LookupError>> = Box::pin(c.lookup("a1".into()));
    let _: MethodStream<'_, Tick, NoProgress, ConformanceEvent> = c.count(3);
    let _: MethodStream<'_, Result<Row, ScanError>, NoProgress, ConformanceEvent> = c.scan("a".into());
    let _: MethodStream<'_, Result<Loaded, LoadError>, LoadStep, ConformanceEvent> = c.load(2);
    let _: MethodStream<'_, Warmed, Note, ConformanceEvent> = c.warm();
}

#[test]
fn the_client_names_every_method() {
    type C = ConformanceClient<MemoryTransport>;
    assert_eq!(C::NAMESPACE, "conformance");
    let methods: Vec<String> = [
        C::METHOD_PING,
        C::METHOD_LOOKUP,
        C::METHOD_COUNT,
        C::METHOD_SCAN,
        C::METHOD_LOAD,
        C::METHOD_WARM,
    ]
    .iter()
    .map(|m| m.qualified())
    .collect();
    assert_eq!(
        methods,
        [
            "conformance.ping",
            "conformance.lookup",
            "conformance.count",
            "conformance.scan",
            "conformance.load",
            "conformance.warm"
        ]
    );
}

// ------------------------------------------------------------------ acceptance 1

#[tokio::test]
async fn ping_returns_its_one_item() {
    let c = client();
    let pong = c.ping().await.expect("the call succeeds");
    assert_eq!(c.transport().last_params(), stated_params("wire/ping.jsonl"));
    assert_eq!(
        vec![pong],
        expected_items("wire/ping.jsonl", serde_json::from_value::<Pong>)
    );
}

#[tokio::test]
async fn lookup_returns_ok_or_err_as_the_activations_answer() {
    let c = client();

    let found = c.lookup("a1".into()).await.expect("the call succeeds");
    assert_eq!(c.transport().last_params(), stated_params("wire/lookup.ok.jsonl"));
    assert_eq!(
        vec![found.clone()],
        expected_items("wire/lookup.ok.jsonl", wire_result::from_value::<Thing, LookupError>)
    );
    assert!(found.is_ok());

    // The activation saying no is a value, not a ClientError.
    let missing = c.lookup("missing".into()).await.expect("the call succeeds");
    assert_eq!(c.transport().last_params(), stated_params("wire/lookup.err.jsonl"));
    assert_eq!(
        vec![missing.clone()],
        expected_items("wire/lookup.err.jsonl", wire_result::from_value::<Thing, LookupError>)
    );
    assert_eq!(missing, Err(LookupError::NotFound { id: "missing".into() }));
}

#[tokio::test]
async fn count_streams_its_items() {
    let c = client();
    let ticks: Vec<Tick> = c.count(3).try_collect().await.expect("the call succeeds");
    assert_eq!(c.transport().last_params(), stated_params("wire/count.jsonl"));
    assert_eq!(ticks, expected_items("wire/count.jsonl", serde_json::from_value::<Tick>));
    assert_eq!(ticks.len(), 3);
}

#[tokio::test]
async fn scan_yields_its_mid_stream_err_and_carries_on() {
    let c = client();
    let rows: Vec<Result<Row, ScanError>> = c.scan("a".into()).try_collect().await.expect("the call succeeds");
    assert_eq!(c.transport().last_params(), stated_params("wire/scan.jsonl"));
    assert_eq!(
        rows,
        expected_items("wire/scan.jsonl", wire_result::from_value::<Row, ScanError>)
    );
    assert!(
        matches!(&rows[..], [Ok(_), Err(ScanError::Unreadable { .. }), Ok(_)]),
        "an err, then more items: {rows:?}"
    );
}

#[tokio::test]
async fn load_yields_its_load_steps_in_order_when_opted_in() {
    let c = client();
    let frames: Vec<Frame<Result<Loaded, LoadError>, LoadStep>> =
        c.load(2).with_progress().try_collect().await.expect("the call succeeds");
    assert_eq!(c.transport().last_params(), stated_params("wire/load.jsonl"));
    assert_eq!(
        frames,
        expected_frames("wire/load.jsonl", wire_result::from_value::<Loaded, LoadError>)
    );
    assert_eq!(
        frames,
        vec![
            Frame::Progress(LoadStep::Batch { done: 1, of: 2 }),
            Frame::Progress(LoadStep::Batch { done: 2, of: 2 }),
            Frame::Item(Ok(Loaded { items: 2 })),
        ]
    );

    // Not opted in: the progress frames are dropped.
    let items: Vec<_> = c.load(2).try_collect().await.expect("the call succeeds");
    assert_eq!(items, vec![Ok(Loaded { items: 2 })]);
}

#[tokio::test]
async fn warm_yields_its_notes_in_order_when_opted_in() {
    let c = client();
    let frames: Vec<Frame<Warmed, Note>> = c.warm().with_progress().try_collect().await.expect("the call succeeds");
    assert_eq!(c.transport().last_params(), stated_params("wire/warm.jsonl"));
    assert_eq!(
        frames,
        expected_frames("wire/warm.jsonl", serde_json::from_value::<Warmed>)
    );
    assert_eq!(
        frames,
        vec![
            Frame::Progress(Note::new("cache cold")),
            Frame::Progress(Note::new("cache warming").with_percentage(50.0)),
            Frame::Item(Warmed { entries: 2 }),
        ]
    );
}

/// `with_progress()` exists on every many-item method; on one that declares
/// none (`P = NoProgress`) it yields only items.
#[tokio::test]
async fn with_progress_on_a_method_without_progress_yields_only_items() {
    let c = client();
    let frames: Vec<Frame<Tick, NoProgress>> = c.count(2).with_progress().try_collect().await.unwrap();
    assert_eq!(frames, vec![Frame::Item(Tick { n: 1 }), Frame::Item(Tick { n: 2 })]);
}

/// Every `errors.jsonl` call surfaces as `ClientError::Remote` carrying that
/// line's `RemoteError`: once through the raw typed path (the calls a client
/// method cannot make — an unknown method, a missing or mistyped param), and
/// for the two the client's own shapes can carry, through `call_one` and
/// `MethodStream` as a generated method would.
#[tokio::test]
async fn every_call_failure_is_a_typed_remote_error() {
    let errors = frames("errors.jsonl");
    let transport = InProcessTransport::new(Conformance);
    let mut seen: BTreeMap<&'static str, RemoteError> = BTreeMap::new();

    let error_calls: Vec<Call> = calls().into_iter().filter(|c| c.line.is_some()).collect();
    assert_eq!(error_calls.len(), 3);
    for call in error_calls {
        let StreamItem::Error { error: expected, .. } = &errors[call.line.unwrap() - 1] else {
            panic!("errors.jsonl line {:?} is an error frame", call.line)
        };
        let method: &'static str = Box::leak(call.method.clone().into_boxed_str());
        let got: Vec<Result<Value, ClientError>> =
            decode::<Value>(transport.call(MethodRef::new("conformance", method), call.params.clone()))
                .collect()
                .await;
        assert_eq!(
            got,
            vec![Err(ClientError::Remote(expected.clone()))],
            "{} {}",
            call.method,
            call.params
        );
        seen.insert(expected.kind(), expected.clone());
    }
    assert_eq!(
        seen.keys().copied().collect::<Vec<_>>(),
        ["invalid_params", "method_not_found"],
        "the kinds errors.jsonl covers"
    );

    // `lookup {}` through a one-item call.
    let StreamItem::Error { error: missing, .. } = &errors[1] else { unreachable!() };
    let got = call_one(
        &transport,
        ConformanceClient::<MemoryTransport>::METHOD_LOOKUP,
        Ok(json!({})),
        wire_result::from_value::<Thing, LookupError>,
    )
    .await;
    assert_eq!(got, Err(ClientError::Remote(missing.clone())));
    assert!(matches!(
        missing,
        RemoteError::InvalidParams { param: Some(p), .. } if p == "id"
    ));

    // `count {"to": "three"}` through a many-item call.
    let StreamItem::Error { error: malformed, .. } = &errors[2] else { unreachable!() };
    let got: Vec<_> = MethodStream::<'_, Tick, NoProgress, ConformanceEvent>::call(
        &transport,
        ConformanceClient::<MemoryTransport>::METHOD_COUNT,
        Ok(json!({"to": "three"})),
        serde_json::from_value::<Tick>,
        ConformanceEvent::Count,
    )
    .collect()
    .await;
    assert_eq!(got, vec![Err(ClientError::Remote(malformed.clone()))]);
}

// ------------------------------------------------------------------ acceptance 2

/// Every item of the `wire/` calls, in `calls.json` order, through the client
/// and into the aggregate: one-item results wrapped in their variant, many-
/// item calls through `.events()`.
async fn every_item_as_an_event(c: &Client) -> Vec<ConformanceEvent> {
    let mut events = vec![
        ConformanceEvent::Ping(c.ping().await.unwrap()),
        ConformanceEvent::Lookup(c.lookup("a1".into()).await.unwrap()),
        ConformanceEvent::Lookup(c.lookup("missing".into()).await.unwrap()),
    ];
    for stream in [
        c.count(3).events(),
        c.scan("a".into()).events(),
        c.load(2).events(),
        c.warm().events(),
    ] {
        let more: Vec<ConformanceEvent> = stream.try_collect().await.unwrap();
        events.extend(more);
    }
    events
}

/// Equal by variant and payload (the aggregate derives `Debug` and `Clone`,
/// not `PartialEq`). Exhaustive: a new method fails to compile here.
fn same(a: &ConformanceEvent, b: &ConformanceEvent) -> bool {
    use ConformanceEvent::*;
    match (a, b) {
        (Ping(x), Ping(y)) => x == y,
        (Lookup(x), Lookup(y)) => x == y,
        (Count(x), Count(y)) => x == y,
        (Scan(x), Scan(y)) => x == y,
        (Load(x), Load(y)) => x == y,
        (Warm(x), Warm(y)) => x == y,
        (Ping(_) | Lookup(_) | Count(_) | Scan(_) | Load(_) | Warm(_), _) => false,
    }
}

#[tokio::test]
async fn the_aggregate_reproduces_aggregate_jsonl_byte_for_byte_and_back() {
    let c = client();
    let events = every_item_as_an_event(&c).await;

    let mut serialized = String::new();
    for ev in &events {
        serialized.push_str(&serde_json::to_string(ev).expect("an event serializes"));
        serialized.push('\n');
    }
    let expected = read("aggregate.jsonl");
    assert_eq!(serialized, expected, "byte for byte");

    let lines: Vec<&str> = expected.lines().collect();
    assert_eq!(lines.len(), events.len());
    for (line, ev) in lines.iter().zip(&events) {
        let back: ConformanceEvent = serde_json::from_str(line).unwrap_or_else(|e| panic!("{e}: {line}"));
        assert!(same(&back, ev), "{line} decodes back to {back:?}, not {ev:?}");
        assert_eq!(serde_json::to_string(&back).unwrap(), *line);
    }
}
