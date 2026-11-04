//! RETURNS-38 · `watch((a, b, c))` on the wire: nothing new.
//!
//! Through the macro-emitted `TicketsClient<&WsTransport>` against the
//! scripted hub (the harness is `ws_transport.rs`'s): a merge of three
//! method streams sends exactly the frames three hand-made streams send —
//! one `menger.call` each, and, dropped while every subscription is open,
//! one `menger.call_unsub` each, naming its own subscription.

// The scripted hub is ws_transport.rs's, whole; this file uses part of it.
#![allow(dead_code)]

use futures::StreamExt;
use plexus_client::{watch, Watched3, WsTransport};
use serde_json::{json, Value};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tickets_api::{TicketEvent, TicketsClient};
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

/// Every subscribe is acknowledged with its own subscription (`sub-<id>`)
/// and one event naming its epic, then left open; an unsubscribe gets its
/// ordinary reply.
fn open_ended() -> Handler {
    Arc::new(|frame: &Value| {
        if !is_subscribe(frame) {
            return vec![Act::Send(json!({"jsonrpc": "2.0", "id": frame["id"], "result": true}).to_string())];
        }
        let sub = json!(format!("sub-{}", frame["id"]));
        let epic = frame["params"]["params"]["epic"].as_str().unwrap_or("-").to_string();
        vec![
            Act::Send(ack(frame, sub.clone())),
            Act::Send(notify("menger.call", sub, data(json!({"type": "not_found", "id": epic})))),
        ]
    })
}

/// What a test compares: each frame's method and what it names — the
/// called method and its params for a subscribe, the subscription for an
/// unsubscribe — in a fixed order (request ids are the transport's own).
fn shape(frames: &[Value]) -> Vec<String> {
    let calls: Vec<Value> = frames.iter().filter(|f| is_subscribe(f)).cloned().collect();
    let mut out: Vec<String> = frames
        .iter()
        .map(|f| {
            if is_subscribe(f) {
                format!("call {} {}", f["params"]["method"], f["params"]["params"])
            } else {
                // which call's subscription this unsubscribes
                let sub = f["params"][0].as_str().unwrap_or_default().to_string();
                let call = calls
                    .iter()
                    .find(|c| format!("sub-{}", c["id"]) == sub)
                    .map(|c| c["params"]["params"].to_string())
                    .unwrap_or_else(|| format!("unknown {sub}"));
                format!("{} {call}", f["method"].as_str().unwrap_or_default())
            }
        })
        .collect();
    out.sort();
    out
}

#[tokio::test]
async fn watch_frames_equal_three_hand_made() {
    // Three hand-made streams, each read once and dropped mid-stream.
    let by_hand = FakeHub::start(open_ended()).await;
    {
        let transport = WsTransport::connect(&by_hand.url(), "menger").await.expect("connect");
        let client = TicketsClient::new(&transport);
        let mut a = client.watch(Some("A".into()));
        let mut b = client.watch(Some("B".into()));
        let mut c = client.watch(Some("C".into()));
        for s in [&mut a, &mut b, &mut c] {
            assert!(matches!(s.next().await, Some(Ok(TicketEvent::NotFound { .. }))));
        }
    }
    let hand_frames = by_hand.wait_for_frames(6).await;

    // The same three, merged.
    let merged = FakeHub::start(open_ended()).await;
    {
        let transport = WsTransport::connect(&merged.url(), "menger").await.expect("connect");
        let client = TicketsClient::new(&transport);
        let mut s = watch((client.watch(Some("A".into())), client.watch(Some("B".into())), client.watch(Some("C".into()))));
        let mut seen = Vec::new();
        while seen.len() < 3 {
            match s.next().await.expect("open") {
                Watched3::A(Ok(TicketEvent::NotFound { id })) => seen.push(("A", id)),
                Watched3::B(Ok(TicketEvent::NotFound { id })) => seen.push(("B", id)),
                Watched3::C(Ok(TicketEvent::NotFound { id })) => seen.push(("C", id)),
                other => panic!("{other:?}"),
            }
        }
        // Each source's item came from its own subscription.
        seen.sort();
        assert_eq!(seen, vec![("A", "A".to_string()), ("B", "B".to_string()), ("C", "C".to_string())]);
        // Dropped here, with all three subscriptions open.
    }
    let merged_frames = merged.wait_for_frames(6).await;
    tokio::time::sleep(Duration::from_millis(100)).await; // nothing more comes

    let merged_frames_now = merged.frames();
    assert_eq!(merged_frames_now.len(), 6, "3 calls + 3 unsubscribes, no more: {merged_frames_now:?}");
    let calls = merged_frames.iter().filter(|f| f["method"] == "menger.call").count();
    let unsubs = merged_frames.iter().filter(|f| f["method"] == "menger.call_unsub").count();
    assert_eq!((calls, unsubs), (3, 3));
    assert_eq!(shape(&merged_frames_now), shape(&hand_frames), "the merge's frames are the hand-made ones");
    assert!(shape(&merged_frames_now).iter().all(|s| !s.contains("unknown")), "each unsubscribe names a live subscription");
}
