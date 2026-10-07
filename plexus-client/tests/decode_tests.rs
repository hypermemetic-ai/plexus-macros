//! RETURNS-9: the client runtime's decoding contract — every wire item
//! shape, and what `MethodStream`, `.with_progress()`, `.events()` and
//! `call_one` make of it. These are the assertions a transport and a
//! generated client are written against.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use futures::{StreamExt, TryStreamExt};
use futures_core::FusedStream;
use plexus_client::{
    call_one, decode, decode_result, decode_with_progress, wire_result, ClientError, Frame,
    ItemStream, MemoryTransport, MethodRef, MethodStream, NoProgress, Note, Params, RemoteError,
    StreamItem, StreamMetadata, Transport, TransportError, TransportErrorKind,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct Tick {
    n: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ScanError {
    Unreadable { key: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum Step {
    Batch { done: u32, of: u32 },
}

/// Stands in for a derived `{Ns}Event`: one variant per method.
#[derive(Debug, Clone, PartialEq)]
enum DemoEvent {
    Count(Tick),
    Scan(Result<Tick, ScanError>),
}

const COUNT: MethodRef = MethodRef::new("demo", "count");
const SCAN: MethodRef = MethodRef::new("demo", "scan");
const PING: MethodRef = MethodRef::new("demo", "ping");

fn data(method: MethodRef, content: Value) -> StreamItem {
    StreamItem::Data {
        metadata: None,
        content_type: Some(method.qualified()),
        content,
    }
}

fn progress(method: MethodRef, content: Value) -> StreamItem {
    StreamItem::Progress {
        metadata: None,
        content_type: Some(method.qualified()),
        content,
    }
}

fn error(error: RemoteError) -> StreamItem {
    StreamItem::Error {
        metadata: Some(StreamMetadata::default()),
        recoverable: false,
        error,
    }
}

fn count_stream(t: &MemoryTransport) -> MethodStream<'_, Tick, Step, DemoEvent> {
    MethodStream::call(
        t,
        COUNT,
        Params::new().build(),
        serde_json::from_value::<Tick>,
        DemoEvent::Count,
    )
}

// ---------------------------------------------------------------- items

#[tokio::test]
async fn data_frames_decode_as_the_item_and_done_ends_the_stream() {
    let t = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }, Tick { n: 2 }]);
    let got: Vec<_> = decode::<Tick>(t.call(COUNT, json!({}))).collect().await;
    assert_eq!(got, vec![Ok(Tick { n: 1 }), Ok(Tick { n: 2 })], "done is not an item");
}

/// A many-item stream is a plain `TryStream`: `.try_collect()` is what
/// MACRO-2's `m_all` was.
#[tokio::test]
async fn a_method_stream_is_a_try_stream() {
    let t = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }, Tick { n: 2 }]);
    let ticks: Vec<Tick> = count_stream(&t).try_collect().await.expect("no error");
    assert_eq!(ticks, vec![Tick { n: 1 }, Tick { n: 2 }]);
}

#[tokio::test]
async fn progress_is_dropped_unless_opted_in() {
    let script = vec![
        progress(COUNT, json!({"type": "batch", "done": 1, "of": 2})),
        data(COUNT, json!({"n": 1})),
        progress(COUNT, json!({"type": "batch", "done": 2, "of": 2})),
        StreamItem::done(),
    ];
    let t = MemoryTransport::new().script(COUNT, script);
    let got: Vec<_> = count_stream(&t).collect().await;
    assert_eq!(got, vec![Ok(Tick { n: 1 })]);
}

#[tokio::test]
async fn with_progress_yields_typed_frames_in_arrival_order() {
    let script = vec![
        progress(COUNT, json!({"type": "batch", "done": 1, "of": 2})),
        data(COUNT, json!({"n": 1})),
        progress(COUNT, json!({"type": "batch", "done": 2, "of": 2})),
        StreamItem::done(),
    ];
    let t = MemoryTransport::new().script(COUNT, script);
    let got: Vec<_> = count_stream(&t).with_progress().collect().await;
    assert_eq!(
        got,
        vec![
            Ok(Frame::Progress(Step::Batch { done: 1, of: 2 })),
            Ok(Frame::Item(Tick { n: 1 })),
            Ok(Frame::Progress(Step::Batch { done: 2, of: 2 })),
        ]
    );
}

#[tokio::test]
async fn the_default_progress_type_is_a_note() {
    let t = MemoryTransport::new().script(
        COUNT,
        vec![
            progress(COUNT, json!({"message": "warming", "percentage": 50.0})),
            data(COUNT, json!({"n": 7})),
            StreamItem::done(),
        ],
    );
    let got: Vec<_> = decode_with_progress::<Tick, Note>(t.call(COUNT, json!({})))
        .collect()
        .await;
    assert_eq!(
        got,
        vec![
            Ok(Frame::Progress(Note::new("warming").with_percentage(50.0))),
            Ok(Frame::Item(Tick { n: 7 })),
        ]
    );
}

/// `NoProgress` is uninhabited: on a method that declares no progress,
/// `.with_progress()` only ever yields items — a progress frame the schema
/// does not declare cannot decode, and says so.
#[tokio::test]
async fn no_progress_never_yields_a_progress_frame() {
    let t = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }]);
    let s: MethodStream<'_, Tick, NoProgress, Tick> =
        MethodStream::new(t.call(COUNT, json!({})), serde_json::from_value::<Tick>, |i| i);
    let got: Vec<_> = s.with_progress().collect().await;
    assert!(matches!(got[..], [Ok(Frame::Item(Tick { n: 1 }))]), "{got:?}");

    let stray = MemoryTransport::new().script(
        COUNT,
        vec![progress(COUNT, json!({"message": "?"})), data(COUNT, json!({"n": 1}))],
    );
    let got: Vec<_> = decode::<Tick>(stray.call(COUNT, json!({})))
        .with_progress()
        .collect()
        .await;
    match &got[..] {
        [Err(ClientError::Decode { content_type, message })] => {
            assert_eq!(content_type.as_deref(), Some("demo.count"));
            assert!(message.contains("declares no progress"), "{message}");
        }
        other => panic!("expected one decode error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_progress_frame_that_does_not_decode_is_a_decode_error() {
    let t = MemoryTransport::new().script(
        COUNT,
        vec![
            progress(COUNT, json!({"type": "batch", "done": "one"})),
            data(COUNT, json!({"n": 1})),
        ],
    );
    let got: Vec<_> = count_stream(&t).with_progress().collect().await;
    assert!(
        matches!(&got[..], [Err(ClientError::Decode { message, .. })] if message.starts_with("progress:")),
        "{got:?}"
    );
}

// ---------------------------------------------------------------- Result items

/// An `err` item is data — the activation saying no — so it is a value and
/// the stream carries on past it.
#[tokio::test]
async fn result_items_decode_ok_and_err_and_an_err_does_not_end_the_stream() {
    let t = MemoryTransport::new().script(
        SCAN,
        vec![
            data(SCAN, json!({"ok": {"n": 1}})),
            data(SCAN, json!({"err": {"type": "unreadable", "key": "a2"}})),
            data(SCAN, json!({"ok": {"n": 3}})),
            StreamItem::done(),
        ],
    );
    let items: Vec<Result<Tick, ScanError>> = decode_result::<Tick, ScanError>(t.call(SCAN, json!({})))
        .try_collect()
        .await
        .expect("no call failure");
    assert_eq!(
        items,
        vec![
            Ok(Tick { n: 1 }),
            Err(ScanError::Unreadable { key: "a2".into() }),
            Ok(Tick { n: 3 }),
        ]
    );
}

#[tokio::test]
async fn a_result_item_without_ok_or_err_is_a_decode_error() {
    let t = MemoryTransport::new().script(SCAN, vec![data(SCAN, json!({"n": 1}))]);
    let got: Vec<_> = decode_result::<Tick, ScanError>(t.call(SCAN, json!({})))
        .collect()
        .await;
    match &got[..] {
        [Err(ClientError::Decode { message, .. })] => {
            assert!(message.contains("`ok`") && message.contains("`err`"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

// ---------------------------------------------------------------- the aggregate

#[tokio::test]
async fn events_wraps_each_item_in_its_methods_variant_and_drops_progress() {
    let t = MemoryTransport::new().script(
        COUNT,
        vec![
            progress(COUNT, json!({"type": "batch", "done": 1, "of": 1})),
            data(COUNT, json!({"n": 1})),
            StreamItem::done(),
        ],
    );
    let got: Vec<DemoEvent> = count_stream(&t).events().try_collect().await.unwrap();
    assert_eq!(got, vec![DemoEvent::Count(Tick { n: 1 })]);
}

/// Every method's `.events()` is one type, so calls merge with ordinary
/// stream plumbing (RETURNS-1 decision 6).
#[tokio::test]
async fn two_methods_events_merge_into_one_listener() {
    let t = MemoryTransport::new()
        .script_events(COUNT, &[Tick { n: 1 }])
        .script(
            SCAN,
            vec![
                data(SCAN, json!({"err": {"type": "unreadable", "key": "k"}})),
                StreamItem::done(),
            ],
        );
    let scan: MethodStream<'_, Result<Tick, ScanError>, NoProgress, DemoEvent> = MethodStream::call(
        &t,
        SCAN,
        Params::new().build(),
        wire_result::from_value::<Tick, ScanError>,
        DemoEvent::Scan,
    );
    let mut all = futures::stream::select_all([count_stream(&t).events(), scan.events()]);
    let mut seen = Vec::new();
    while let Some(ev) = all.next().await {
        seen.push(ev.expect("no call failure"));
    }
    assert_eq!(seen.len(), 2);
    assert!(seen.contains(&DemoEvent::Count(Tick { n: 1 })));
    assert!(seen.contains(&DemoEvent::Scan(Err(ScanError::Unreadable { key: "k".into() }))));
}

// ---------------------------------------------------------------- call failures

#[tokio::test]
async fn an_error_frame_is_a_typed_remote_error_and_ends_the_stream() {
    let t = MemoryTransport::new().script(
        COUNT,
        vec![
            data(COUNT, json!({"n": 1})),
            error(RemoteError::Execution {
                message: "boom".into(),
            }),
            // Anything after a terminal item must not be delivered.
            data(COUNT, json!({"n": 2})),
        ],
    );
    let mut s = count_stream(&t);
    assert_eq!(s.next().await, Some(Ok(Tick { n: 1 })));
    let err = s.next().await.expect("an item").unwrap_err();
    assert_eq!(
        err,
        ClientError::Remote(RemoteError::Execution {
            message: "boom".into()
        })
    );
    assert!(err.is_remote() && !err.is_transport());
    assert_eq!(err.remote().map(RemoteError::kind), Some("execution"));
    assert_eq!(err.to_string(), "remote error: execution failed: boom");
    assert_eq!(s.next().await, None);
    assert!(s.is_terminated());
}

#[tokio::test]
async fn a_transport_failure_is_a_transport_error_and_ends_the_stream() {
    let t = MemoryTransport::new().script_failure(COUNT, TransportError::connect("refused"));
    let got: Vec<_> = count_stream(&t).collect().await;
    match &got[..] {
        [Err(e @ ClientError::Transport(TransportError { kind: TransportErrorKind::Connect, .. }))] => {
            assert!(e.is_transport() && !e.is_remote())
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_data_item_that_does_not_decode_is_a_decode_error_and_ends_the_stream() {
    let t = MemoryTransport::new().script(
        COUNT,
        vec![data(COUNT, json!({"n": "one"})), data(COUNT, json!({"n": 2}))],
    );
    let got: Vec<_> = count_stream(&t).collect().await;
    match &got[..] {
        [Err(ClientError::Decode { content_type, .. })] => {
            assert_eq!(content_type.as_deref(), Some("demo.count"))
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn bidirectional_request_items_are_skipped() {
    let t = MemoryTransport::new().script(
        COUNT,
        vec![
            StreamItem::Request {
                request_id: "r1".into(),
                request_data: json!({"prompt": "continue?"}),
                timeout_ms: 1000,
            },
            data(COUNT, json!({"n": 1})),
            StreamItem::done(),
        ],
    );
    let ticks: Vec<Tick> = count_stream(&t).try_collect().await.unwrap();
    assert_eq!(ticks, vec![Tick { n: 1 }]);
}

/// A parameter that will not encode is reported as the stream's one item
/// and nothing is sent.
#[tokio::test]
async fn an_encode_failure_is_yielded_once_and_nothing_is_sent() {
    let t = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }]);
    let params = Params::new().param("to", FailsToSerialize).build();
    let s: MethodStream<'_, Tick> =
        MethodStream::call(&t, COUNT, params, serde_json::from_value::<Tick>, |i| i);
    let got: Vec<_> = s.collect().await;
    match &got[..] {
        [Err(ClientError::Encode { param: "to", .. })] => {}
        other => panic!("{other:?}"),
    }
    assert!(t.calls().is_empty(), "nothing was sent");
}

struct FailsToSerialize;
impl Serialize for FailsToSerialize {
    fn serialize<S: serde::Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
        Err(serde::ser::Error::custom("not on the wire"))
    }
}

// ---------------------------------------------------------------- one-item calls

async fn ping(t: &dyn Transport) -> Result<Tick, ClientError> {
    call_one(t, PING, Params::new().build(), serde_json::from_value::<Tick>).await
}

#[tokio::test]
async fn call_one_returns_the_one_item() {
    let t = MemoryTransport::new().script_events(PING, &[Tick { n: 9 }]);
    assert_eq!(ping(&t).await, Ok(Tick { n: 9 }));
}

#[tokio::test]
async fn call_one_with_no_item_or_two_is_a_contract_break() {
    let none = MemoryTransport::new().script_events::<Tick>(PING, &[]);
    match ping(&none).await {
        Err(ClientError::Decode { content_type, message }) => {
            assert_eq!(content_type.as_deref(), Some("demo.ping"));
            assert!(message.contains("without its item"), "{message}");
        }
        other => panic!("{other:?}"),
    }

    let two = MemoryTransport::new().script_events(PING, &[Tick { n: 1 }, Tick { n: 2 }]);
    match ping(&two).await {
        Err(ClientError::Decode { message, .. }) => {
            assert!(message.contains("more than one"), "{message}")
        }
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn call_one_surfaces_an_error_frame_before_or_after_the_item() {
    let before = MemoryTransport::new().script(PING, vec![error(RemoteError::Unauthenticated)]);
    assert_eq!(
        ping(&before).await,
        Err(ClientError::Remote(RemoteError::Unauthenticated))
    );

    let after = MemoryTransport::new().script(
        PING,
        vec![data(PING, json!({"n": 1})), error(RemoteError::Unauthenticated)],
    );
    assert_eq!(
        ping(&after).await,
        Err(ClientError::Remote(RemoteError::Unauthenticated))
    );
}

#[tokio::test]
async fn call_one_with_an_unencodable_param_sends_nothing() {
    let t = MemoryTransport::new().script_events(PING, &[Tick { n: 1 }]);
    let got = call_one(
        &t,
        PING,
        Params::new().param("x", FailsToSerialize).build(),
        serde_json::from_value::<Tick>,
    )
    .await;
    assert!(matches!(got, Err(ClientError::Encode { param: "x", .. })), "{got:?}");
    assert!(t.calls().is_empty());
}

/// A transport that notes when its stream is read past `done` — i.e. when
/// the caller drove the call to its end instead of dropping it early.
struct Watched {
    inner: MemoryTransport,
    drained: Arc<AtomicBool>,
}

impl Transport for Watched {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        let drained = self.drained.clone();
        let items = self.inner.call(method, params).inspect(move |item| {
            if matches!(item, Ok(StreamItem::Done { .. })) {
                drained.store(true, Ordering::SeqCst);
            }
        });
        Box::pin(items)
    }
}

/// A one-item call reads to its `done`, so a transport that cancels a
/// dropped call (the WebSocket one unsubscribes) never cancels one that
/// finished.
#[tokio::test]
async fn call_one_reads_the_call_to_its_done() {
    let t = Watched {
        inner: MemoryTransport::new().script_events(PING, &[Tick { n: 1 }]),
        drained: Arc::new(AtomicBool::new(false)),
    };
    assert_eq!(ping(&t).await, Ok(Tick { n: 1 }));
    assert!(t.drained.load(Ordering::SeqCst), "the done frame was read");
}

// ---------------------------------------------------------------- params

#[test]
fn params_omit_a_none_option_and_keep_the_rest_by_name() {
    let p = Params::new()
        .param("id", "A-1")
        .optional("epic", Some("MACRO"))
        .optional("parent", None::<String>)
        .build()
        .unwrap();
    assert_eq!(p, json!({"id": "A-1", "epic": "MACRO"}));
    assert_eq!(Params::new().build().unwrap(), json!({}));
}

#[test]
fn the_first_unencodable_param_is_kept() {
    let err = Params::new()
        .param("a", FailsToSerialize)
        .param("b", 1)
        .build()
        .unwrap_err();
    assert!(matches!(err, ClientError::Encode { param: "a", .. }), "{err:?}");
}

// ---------------------------------------------------------------- the transport seam

#[tokio::test]
async fn an_unscripted_method_is_a_transport_protocol_error() {
    let t = MemoryTransport::new();
    let err = ping(&t).await.unwrap_err();
    assert!(err.is_transport(), "{err:?}");
}

#[tokio::test]
async fn the_transport_records_the_params_it_was_handed() {
    let t = MemoryTransport::new().script_events(COUNT, &[Tick { n: 1 }]);
    let s: MethodStream<'_, Tick> = MethodStream::call(
        &t,
        COUNT,
        Params::new().param("epic", "MACRO").build(),
        serde_json::from_value::<Tick>,
        |i| i,
    );
    let _: Vec<_> = s.collect().await;
    assert_eq!(t.calls(), vec![("demo.count".to_string(), json!({"epic": "MACRO"}))]);
    assert_eq!(t.params_of(COUNT), Some(json!({"epic": "MACRO"})));
}

#[test]
fn method_ref_keeps_the_namespace_and_the_method_separable() {
    assert_eq!(COUNT.namespace(), "demo");
    assert_eq!(COUNT.method(), "count");
    assert_eq!(COUNT.qualified(), "demo.count");
    assert_eq!(COUNT.to_string(), "demo.count");
}
