//! MACRO-6 · the WebSocket JSON-RPC transport, pinned frame by frame.
//!
//! These tests speak to a **scripted** JSON-RPC server in this file rather
//! than to a real server, because the properties under test are properties of the
//! *transport* and several of them a real server will not reproduce on
//! demand: a notification under a method name the server does not currently
//! use, a socket that dies mid-stream, a JSON-RPC `error` answer to a
//! subscribe. `wire_over_websocket.rs` is the other half — the same
//! transport against a real menger.
//!
//! The scripted server records every frame the client sends, so each test
//! asserts on the actual bytes.

use futures::{StreamExt, TryStreamExt};
use tickets_api::{TicketEvent, TicketsClient};
use plexus_client::{
    ClientError, MethodRef, RemoteError, StreamItem, Transport, TransportErrorKind, WsTransport,
};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio_tungstenite::tungstenite::Message;

// ---------------------------------------------------------------------------
// The scripted server
// ---------------------------------------------------------------------------

/// What the scripted server does in response to one inbound frame.
enum Act {
    Send(String),
    /// Hold the reply back, so a test can pin an ordering the client would
    /// otherwise win by a few microseconds.
    Delay(Duration),
    Close,
}

type Handler = Arc<dyn Fn(&Value) -> Vec<Act> + Send + Sync>;

struct FakeHub {
    addr: SocketAddr,
    /// Every frame the client sent, in order.
    seen: Arc<Mutex<Vec<Value>>>,
    /// Set when the client's end of the socket goes away.
    hung_up: Arc<AtomicBool>,
}

impl FakeHub {
    async fn start(handler: Handler) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let seen: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
        let hung_up = Arc::new(AtomicBool::new(false));

        let seen_task = seen.clone();
        let hung_up_task = hung_up.clone();
        tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept");
            let ws = tokio_tungstenite::accept_async(tcp).await.expect("handshake");
            let (mut out, mut inbound) = ws.split();

            while let Some(Ok(msg)) = inbound.next().await {
                let Message::Text(text) = msg else { continue };
                let frame: Value = serde_json::from_str(text.as_str()).expect("client sent JSON");
                seen_task.lock().unwrap().push(frame.clone());

                for act in handler(&frame) {
                    match act {
                        Act::Send(reply) => {
                            use futures::SinkExt;
                            if out.send(Message::text(reply)).await.is_err() {
                                return;
                            }
                        }
                        Act::Delay(d) => tokio::time::sleep(d).await,
                        Act::Close => {
                            use futures::SinkExt;
                            let _ = out.close().await;
                            return;
                        }
                    }
                }
            }
            hung_up_task.store(true, Ordering::SeqCst);
        });

        Self {
            addr,
            seen,
            hung_up,
        }
    }

    fn url(&self) -> String {
        format!("ws://{}", self.addr)
    }

    fn frames(&self) -> Vec<Value> {
        self.seen.lock().unwrap().clone()
    }

    /// Wait until the client has sent `n` frames (or give up).
    async fn wait_for_frames(&self, n: usize) -> Vec<Value> {
        for _ in 0..200 {
            let frames = self.frames();
            if frames.len() >= n {
                return frames;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("only {} frames arrived, wanted {n}", self.frames().len());
    }
}

/// `{"jsonrpc":"2.0","id":N,"result":<sub>}` — the reply whose `result` *is*
/// the subscription id.
fn ack(frame: &Value, subscription: Value) -> String {
    json!({"jsonrpc": "2.0", "id": frame["id"], "result": subscription}).to_string()
}

/// A stream item, wrapped as the notification the hub emits.
fn notify(method: &str, subscription: Value, item: Value) -> String {
    json!({
        "jsonrpc": "2.0",
        "method": method,
        "params": {"subscription": subscription, "result": item},
    })
    .to_string()
}

fn data(content: Value) -> Value {
    json!({
        "type": "data",
        "metadata": {"provenance": ["menger", "tickets"], "plexus_hash": "abc", "timestamp": 1},
        "content_type": "tickets.watch",
        "content": content,
    })
}

fn done() -> Value {
    json!({"type": "done", "metadata": {"provenance": [], "plexus_hash": "abc", "timestamp": 1}})
}

fn is_subscribe(frame: &Value) -> bool {
    frame["method"] == "menger.call"
}

// ---------------------------------------------------------------------------
// The subscribe frame
// ---------------------------------------------------------------------------

/// A client method call is exactly one `{hub}.call` subscribe frame whose
/// `params.method` is the qualified activation method, and whose
/// `params.params` is the by-name object the generated client built.
#[tokio::test]
async fn a_client_call_is_one_subscribe_frame() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        vec![
            Act::Send(ack(frame, json!(7))),
            Act::Send(notify("result", json!(7), done())),
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    assert_eq!(transport.hub_namespace(), "menger");
    let client = TicketsClient::new(transport);

    let events: Vec<TicketEvent> = client
        .list(Some("MACRO".into()), None)
        .try_collect()
        .await
        .expect("call completes");
    assert!(events.is_empty(), "the script sent no data items");

    let frames = hub.wait_for_frames(1).await;
    assert_eq!(frames.len(), 1, "one call, one frame: {frames:?}");
    assert_eq!(frames[0]["jsonrpc"], "2.0");
    assert_eq!(frames[0]["method"], "menger.call");
    assert_eq!(frames[0]["params"]["method"], "tickets.list");
    // `Option` params that are `None` are omitted, not nulled.
    assert_eq!(frames[0]["params"]["params"], json!({"epic": "MACRO"}));
    println!("subscribe frame: {}", frames[0]);
}

// ---------------------------------------------------------------------------
// The notification method name — the whole reason this transport is hand-written
// ---------------------------------------------------------------------------

/// `PLEXUS_NOTIF_METHOD` is `"result"`, but plexus-core 0.5.3 emits
/// notifications under the subscribe method's own name (`"menger.call"`);
/// `menger/web/index.html` accepts both and says so in a comment. Routing
/// keys on `params.subscription`, so both spellings deliver — and a third
/// spelling nobody has seen yet would too.
#[tokio::test]
async fn both_notification_method_names_route_by_subscription_id() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        let sub = json!(11);
        vec![
            Act::Send(ack(frame, sub.clone())),
            // the documented name …
            Act::Send(notify(
                "result",
                sub.clone(),
                data(json!({"type": "not_found", "id": "UNDER-RESULT"})),
            )),
            // … and the one plexus-core 0.5.3 actually emits.
            Act::Send(notify(
                "menger.call",
                sub.clone(),
                data(json!({"type": "not_found", "id": "UNDER-HUB-CALL"})),
            )),
            // … and one nobody documents, to prove the method name is not
            // consulted at all.
            Act::Send(notify(
                "wibble",
                sub.clone(),
                data(json!({"type": "not_found", "id": "UNDER-WIBBLE"})),
            )),
            Act::Send(notify("result", sub, done())),
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let client = TicketsClient::new(transport);

    let ids: Vec<String> = client
        .watch(None)
        .try_collect::<Vec<_>>()
        .await
        .expect("stream completes")
        .into_iter()
        .map(|e| match e {
            TicketEvent::NotFound { id } => id,
            other => panic!("unexpected event {other:?}"),
        })
        .collect();

    assert_eq!(ids, vec!["UNDER-RESULT", "UNDER-HUB-CALL", "UNDER-WIBBLE"]);
}

// ---------------------------------------------------------------------------
// Unsubscribe on drop
// ---------------------------------------------------------------------------

/// `web/index.html` never unsubscribes and leaks a subscription per reload.
/// Dropping the stream this transport returns sends `{hub}.call_unsub` with
/// the subscription id as the single positional parameter.
#[tokio::test]
async fn dropping_a_watch_stream_sends_call_unsub() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        if !is_subscribe(frame) {
            // The unsubscribe itself gets an ordinary JSON-RPC reply.
            return vec![Act::Send(
                json!({"jsonrpc": "2.0", "id": frame["id"], "result": true}).to_string(),
            )];
        }
        let sub = json!("sub-abc");
        vec![
            Act::Send(ack(frame, sub.clone())),
            Act::Send(notify(
                "menger.call",
                sub,
                data(json!({"type": "not_found", "id": "STILL-OPEN"})),
            )),
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let client = TicketsClient::new(transport);

    {
        let mut watch = client.watch(Some("MACRO".into()));
        let first = watch.next().await.expect("one event").expect("no error");
        assert!(matches!(first, TicketEvent::NotFound { .. }));
        // `watch` is open-ended: the server sent no `done`, so leaving this
        // scope is a mid-stream drop.
    }

    let frames = hub.wait_for_frames(2).await;
    assert_eq!(frames[0]["method"], "menger.call");
    assert_eq!(frames[1]["method"], "menger.call_unsub");
    assert_eq!(
        frames[1]["params"],
        json!(["sub-abc"]),
        "the subscription id goes back positionally"
    );
    println!("unsubscribe frame: {}", frames[1]);
}

/// The mirror image: a stream that ran to `done` has nothing to cancel, so
/// no unsubscribe is sent. (Sending one would be answered with "unknown
/// subscription" and would make every finished call log a server error.)
///
/// RETURNS-9: this is why a one-item call (`get` is `-> TicketEvent`) reads
/// past its item to the `done` before it returns — returning at the item
/// would drop a live subscription and unsubscribe a call that finished.
#[tokio::test]
async fn a_stream_that_reached_done_does_not_unsubscribe() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        let sub = json!(3);
        vec![
            Act::Send(ack(frame, sub.clone())),
            Act::Send(notify(
                "result",
                sub.clone(),
                data(json!({"type": "not_found", "id": "MACRO-6"})),
            )),
            Act::Send(notify("result", sub, done())),
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let client = TicketsClient::new(transport);
    let event = client.get("MACRO-6".into()).await.expect("completes");
    assert_eq!(event, TicketEvent::NotFound { id: "MACRO-6".into() });

    tokio::time::sleep(Duration::from_millis(100)).await;
    let frames = hub.frames();
    assert_eq!(frames.len(), 1, "only the subscribe: {frames:?}");
}

/// The awkward ordering, made deterministic: the subscribe goes out with
/// `call`, so a stream dropped **before it is ever polled** has still opened
/// a subscription — whose id arrives after the caller is gone. The
/// connection's reader is then the only one who can cancel it, and does.
#[tokio::test]
async fn a_stream_dropped_before_the_first_poll_still_unsubscribes() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        if !is_subscribe(frame) {
            return vec![];
        }
        vec![
            // The caller drops in this window.
            Act::Delay(Duration::from_millis(150)),
            Act::Send(ack(frame, json!("late-sub"))),
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let client = TicketsClient::new(transport);

    // Not awaited, not polled — but the subscribe frame is already written.
    drop(client.watch(None));
    hub.wait_for_frames(1).await;

    let frames = hub.wait_for_frames(2).await;
    assert_eq!(frames[0]["method"], "menger.call");
    assert_eq!(frames[1]["method"], "menger.call_unsub");
    assert_eq!(frames[1]["params"], json!(["late-sub"]));
}

// ---------------------------------------------------------------------------
// The two failure modes
// ---------------------------------------------------------------------------

/// A socket that dies mid-stream is a `TransportError` of kind `Closed` on
/// every open stream — not a stream that quietly stops, which a caller
/// cannot tell from a normal end.
#[tokio::test]
async fn a_socket_closing_mid_stream_is_a_closed_transport_error() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        let sub = json!(5);
        vec![
            Act::Send(ack(frame, sub.clone())),
            Act::Send(notify(
                "result",
                sub,
                data(json!({"type": "not_found", "id": "BEFORE-THE-CLOSE"})),
            )),
            // No `done`: the connection just goes away.
            Act::Close,
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let mut items = transport.call(MethodRef::new("tickets", "watch"), json!({}));

    match items.next().await {
        Some(Ok(StreamItem::Data { .. })) => {}
        other => panic!("expected the data item first, got {other:?}"),
    }
    match items.next().await {
        Some(Err(e)) => assert_eq!(e.kind, TransportErrorKind::Closed, "{e}"),
        other => panic!("expected a Closed transport error, got {other:?}"),
    }
    drop(items);
    assert!(transport.is_closed());
}

/// A JSON-RPC `error` answer to the subscribe is a `Protocol` error: the
/// socket is fine, the hub refused the call. Distinct from an in-band
/// `error` *item*, which is the activation's own answer and stays data.
#[tokio::test]
async fn a_json_rpc_error_reply_to_the_subscribe_is_a_protocol_error() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        vec![Act::Send(
            json!({
                "jsonrpc": "2.0",
                "id": frame["id"],
                "error": {"code": -32601, "message": "Method not found"},
            })
            .to_string(),
        )]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let mut items = transport.call(MethodRef::new("nope", "nope"), json!({}));

    match items.next().await {
        Some(Err(e)) => {
            assert_eq!(e.kind, TransportErrorKind::Protocol, "{e}");
            assert!(e.message.contains("-32601"), "{e}");
        }
        other => panic!("expected a Protocol transport error, got {other:?}"),
    }
    assert!(items.next().await.is_none(), "the stream ends there");
}

/// The typed layer keeps the two apart: the same call over the same
/// transport reports `is_transport()` for the socket and `is_remote()` for
/// an in-band `error` item — which arrives typed, as the frame's
/// `RemoteError`.
#[tokio::test]
async fn an_in_band_error_item_is_remote_not_transport() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        let sub = json!(1);
        vec![
            Act::Send(ack(frame, sub.clone())),
            Act::Send(notify(
                "result",
                sub,
                json!({
                    "type": "error",
                    "recoverable": false,
                    "error": {"kind": "execution", "message": "cycle"}
                }),
            )),
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let client = TicketsClient::new(transport);
    let err = client
        .get("CY-1".into())
        .await
        .expect_err("the hub answered error");
    assert!(err.is_remote(), "{err}");
    assert!(!err.is_transport(), "{err}");
    assert_eq!(
        err,
        ClientError::Remote(RemoteError::Execution {
            message: "cycle".into()
        })
    );
}

// ---------------------------------------------------------------------------
// Multiplexing
// ---------------------------------------------------------------------------

/// The reason this transport exists rather than one connection per call:
/// a frontend holds `watch` open while it issues `list`. Two calls, one
/// socket, two independent streams — and the interleaving on the wire does
/// not disturb either.
#[tokio::test]
async fn two_concurrent_calls_are_independent_streams_on_one_connection() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        let qualified = frame["params"]["method"].as_str().unwrap_or_default().to_string();
        match qualified.as_str() {
            // The long-lived one: acknowledged, then nothing until later.
            "tickets.watch" => vec![Act::Send(ack(frame, json!("watch-sub")))],
            // The unary one: acknowledged, answered, and interleaved with
            // an item for the *other* subscription.
            "tickets.list" => vec![
                Act::Send(ack(frame, json!("list-sub"))),
                Act::Send(notify(
                    "menger.call",
                    json!("watch-sub"),
                    data(json!({"type": "not_found", "id": "TO-THE-WATCHER"})),
                )),
                Act::Send(notify(
                    "result",
                    json!("list-sub"),
                    data(json!({"type": "not_found", "id": "TO-THE-LISTER"})),
                )),
                Act::Send(notify("result", json!("list-sub"), done())),
            ],
            other => panic!("unexpected method {other}"),
        }
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    let client = TicketsClient::new(transport);

    let mut watch = client.watch(None);
    // The subscribe for `watch` has to be on the wire before `list` is sent,
    // or the script's ordering is not what is being tested.
    hub.wait_for_frames(1).await;

    let listed: Vec<TicketEvent> = client
        .list(None, None)
        .try_collect()
        .await
        .expect("list completes");
    assert_eq!(
        listed,
        vec![TicketEvent::NotFound {
            id: "TO-THE-LISTER".into()
        }],
        "the lister sees only its own items"
    );

    match watch.next().await {
        Some(Ok(TicketEvent::NotFound { id })) => {
            assert_eq!(id, "TO-THE-WATCHER", "the watcher sees only its own items");
        }
        other => panic!("unexpected watch event {other:?}"),
    }

    let frames = hub.frames();
    assert_eq!(frames.len(), 2, "two subscribes, one connection: {frames:?}");
    assert_eq!(frames[0]["params"]["method"], "tickets.watch");
    assert_eq!(frames[1]["params"]["method"], "tickets.list");
    assert_ne!(frames[0]["id"], frames[1]["id"], "distinct JSON-RPC ids");
}

// ---------------------------------------------------------------------------
// Connecting
// ---------------------------------------------------------------------------

/// Dropping the last handle shuts the socket down. Without this the
/// connection outlives its transport: the reader task holds a writer handle
/// of its own, so the writer's queue never ends by itself.
#[tokio::test]
async fn dropping_the_transport_closes_the_socket() {
    let hub = FakeHub::start(Arc::new(|frame: &Value| {
        let sub = json!(1);
        vec![
            Act::Send(ack(frame, sub.clone())),
            Act::Send(notify(
                "result",
                sub.clone(),
                data(json!({"type": "health", "version": "0.7.0", "ticket_count": 0})),
            )),
            Act::Send(notify("result", sub, done())),
        ]
    }))
    .await;

    let transport = WsTransport::connect(&hub.url(), "menger").await.expect("connect");
    TicketsClient::new(&transport)
        .health()
        .await
        .expect("one call, so the connection is certainly up");
    assert!(!hub.hung_up.load(Ordering::SeqCst));

    drop(transport);
    for _ in 0..100 {
        if hub.hung_up.load(Ordering::SeqCst) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the socket was still open a second after the transport was dropped");
}

/// Nothing listening is a `Connect` error, reported by `connect` rather than
/// by the first call — a consumer that cannot reach the server should learn
/// it at construction.
#[tokio::test]
async fn connecting_to_nothing_is_a_connect_error() {
    // Port 1 on loopback: privileged, and nothing binds it in a test env.
    let err = WsTransport::connect("ws://127.0.0.1:1", "menger")
        .await
        .expect_err("nothing is listening");
    assert_eq!(err.kind, TransportErrorKind::Connect, "{err}");
}
