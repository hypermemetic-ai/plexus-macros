//! RETURNS-38 · `watch((a, b, c))` against a running menger, over
//! `WsTransport` (live check, not a test).
//!
//! ```sh
//! cargo run -p plexus-client --features ws-native --example watch_live -- ws://127.0.0.1:4486
//! ```
//!
//! The URL is a socket that admits the caller — on this machine a throwaway
//! menger behind hub-codegen's `tests/ts_watch/live/frame-proxy.mjs`, which
//! signs the upgrade in and prints every frame the client sends. It needs a
//! scope `LIVE-1` there. Prints what arrived, in order; the proxy says what
//! went on the wire.

use futures::StreamExt;
use plexus_client::{collect_data, decode, watch, ClientEvent, MethodRef, Transport, Watched3, WsTransport};
use serde_json::{json, Value};
use std::time::Duration;

const WATCH: MethodRef = MethodRef::new("tickets", "watch");
const LIST: MethodRef = MethodRef::new("tickets", "list");
const GET: MethodRef = MethodRef::new("tickets", "get");
const AWAIT_REPLY: MethodRef = MethodRef::new("tickets", "await_reply");
const CREATE: MethodRef = MethodRef::new("tickets", "create");

fn kind(ev: &Value) -> String {
    ev["type"].as_str().unwrap_or("?").to_string()
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let url = std::env::args().nth(1).unwrap_or_else(|| "ws://127.0.0.1:4486".into());
    let t = WsTransport::connect(&url, "menger").await?;
    let create = |title: &str| {
        collect_data(decode::<Value>(t.call(CREATE, json!({"epic": "LIVE", "parent": "LIVE-1", "title": title}))))
    };

    // 1. order, end, error: an open watch, a finite list, a get that fails.
    let mut s = watch((
        decode::<Value>(t.call(WATCH, json!({"epic": "LIVE"}))),
        decode::<Value>(t.call(LIST, json!({"epic": "LIVE"}))),
        decode::<Value>(t.call(GET, json!({"id": "not an id!"}))),
    ));
    let mut arrived = Vec::new();
    let mut created = false;
    while let Some(it) = s.next().await {
        let line = match &it {
            Watched3::A(Ok(ClientEvent::Data(ev))) => format!("tickets.watch: {}", kind(ev)),
            Watched3::B(Ok(ClientEvent::Data(ev))) => format!("tickets.list: {}", kind(ev)),
            Watched3::C(Ok(ClientEvent::Data(ev))) => format!("tickets.get: {}", kind(ev)),
            Watched3::A(Ok(ClientEvent::Progress { .. }))
            | Watched3::B(Ok(ClientEvent::Progress { .. }))
            | Watched3::C(Ok(ClientEvent::Progress { .. })) => continue,
            Watched3::A(Err(e)) => format!("tickets.watch: failed ({e})"),
            Watched3::B(Err(e)) => format!("tickets.list: failed ({e})"),
            Watched3::C(Err(e)) => format!("tickets.get: failed ({})", e.to_string().chars().take(50).collect::<String>()),
        };
        arrived.push(line);
        if !created && arrived.iter().any(|a| a.starts_with("tickets.get")) {
            created = true;
            create("watch live (rust)").await?;
        }
        if arrived.iter().any(|a| a == "tickets.watch: created") {
            break;
        }
    }
    println!("live 1 (order, end, error), live streams left: {}", s.live());
    for a in &arrived {
        println!("  {a}");
    }
    drop(s); // the watch is the one still open: one call_unsub
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 2. three open-ended streams merged, then dropped: three call_unsub.
    let mut m = watch((
        decode::<Value>(t.call(WATCH, json!({"epic": "LIVE"}))),
        decode::<Value>(t.call(AWAIT_REPLY, json!({"id": "LIVE-1"}))),
        decode::<Value>(t.call(WATCH, json!({"filter": "under LIVE-1"}))),
    ));
    let first = m.next().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    create("watch live (rust) 2").await?;
    let second = m.next().await;
    let third = m.next().await;
    println!("live 2 (three open, merged): {} items, then dropped with {} live", [first, second, third].iter().filter(|x| x.is_some()).count(), m.live());
    drop(m);
    tokio::time::sleep(Duration::from_millis(300)).await;

    // 3. the same three by hand, each read once, then dropped.
    let mut a = decode::<Value>(t.call(WATCH, json!({"epic": "LIVE"})));
    let mut b = decode::<Value>(t.call(AWAIT_REPLY, json!({"id": "LIVE-1"})));
    let mut c = decode::<Value>(t.call(WATCH, json!({"filter": "under LIVE-1"})));
    let _ = b.next().await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    create("watch live (rust) 3").await?;
    let _ = a.next().await;
    let _ = c.next().await;
    drop((a, b, c));
    tokio::time::sleep(Duration::from_millis(300)).await;
    println!("live 3 (three by hand): dropped");
    Ok(())
}
