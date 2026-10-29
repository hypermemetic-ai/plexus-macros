//! FRAME-16: `WsTransport::adopt` — the `ws-web` backend over a socket made
//! before it (a page opens one before its wasm is instantiated). Run under
//! Node's own `WebSocket`, against a stub that speaks the Plexus frames:
//!
//!   cargo test -p plexus-client --features ws-web --target wasm32-unknown-unknown --test ws_web_adopt
//!
//! (with `CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner`
//! at the version Cargo.lock pins).
#![cfg(all(target_arch = "wasm32", feature = "ws-web"))]

use futures::StreamExt;
use plexus_client::{MethodRef, StreamItem, Transport, WsTransport};
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen_test::*;

wasm_bindgen_test_configure!(run_in_node_experimental);

#[wasm_bindgen(inline_js = r#"
// A stub service `svc`: each `svc.call` is answered with its subscription
// id, one data item echoing its params, then done.
export async function serve() {
  const http = await import("node:http");
  const crypto = await import("node:crypto");
  const frame = (text) => {
    const body = Buffer.from(text);
    const n = body.length;
    const head = n < 126 ? Buffer.from([0x81, n]) : Buffer.from([0x81, 126, n >> 8, n & 255]);
    return Buffer.concat([head, body]);
  };
  const server = http.createServer((_, res) => res.writeHead(426).end());
  let next = 1;
  server.on("upgrade", (req, sock) => {
    const accept = crypto.createHash("sha1").update(req.headers["sec-websocket-key"] + "258EAFA5-E914-47DA-95CA-C5AB0DC85B11").digest("base64");
    sock.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
    let buf = Buffer.alloc(0);
    sock.on("error", () => {});
    sock.on("data", (d) => {
      buf = Buffer.concat([buf, d]);
      for (;;) {
        if (buf.length < 2) return;
        const op = buf[0] & 15;
        let n = buf[1] & 127, at = 2;
        if (n === 126) { if (buf.length < 4) return; n = buf.readUInt16BE(2); at = 4; }
        const mask = buf.subarray(at, at + 4); at += 4;
        if (buf.length < at + n) return;
        const body = Buffer.from(buf.subarray(at, at + n));
        for (let i = 0; i < n; i++) body[i] ^= mask[i & 3];
        buf = buf.subarray(at + n);
        if (op === 8) { sock.end(); return; }
        const req = JSON.parse(body.toString("utf8"));
        if (req.method !== "svc.call") { sock.write(frame(JSON.stringify({ jsonrpc: "2.0", id: req.id, result: true }))); continue; }
        const sub = next++;
        const send = (o) => sock.write(frame(JSON.stringify(o)));
        send({ jsonrpc: "2.0", id: req.id, result: sub });
        const item = (result) => send({ jsonrpc: "2.0", method: "svc.call", params: { subscription: sub, result } });
        item({ type: "data", content_type: req.params.method, content: { echo: req.params.params } });
        item({ type: "done" });
      }
    });
  });
  await new Promise((r) => server.listen(0, "127.0.0.1", r));
  server.unref();
  return server.address().port;
}
export function socket(port) { return new WebSocket(`ws://127.0.0.1:${port}`); }
export function opened(ws) {
  return new Promise((resolve, reject) => {
    if (ws.readyState === 1) return resolve();
    ws.addEventListener("open", () => resolve());
    ws.addEventListener("error", () => reject(new Error("the socket failed")));
  });
}
export function closed(ws) {
  return new Promise((resolve) => {
    if (ws.readyState === 3) return resolve();
    ws.addEventListener("close", () => resolve());
    ws.close();
  });
}
"#)]
extern "C" {
    async fn serve() -> JsValue;
    fn socket(port: u16) -> web_sys::WebSocket;
    async fn opened(ws: &web_sys::WebSocket);
    async fn closed(ws: &web_sys::WebSocket);
}

async fn port() -> u16 {
    serve().await.as_f64().expect("a port") as u16
}

/// One call through `t`: its data's content.
async fn echoed(t: &WsTransport, n: u32) -> serde_json::Value {
    let items: Vec<_> = t.call(MethodRef::new("things", "list"), json!({ "n": n })).collect().await;
    let data: Vec<_> = items
        .into_iter()
        .filter_map(|i| match i.expect("no transport error") {
            StreamItem::Data { content, .. } => Some(content),
            _ => None,
        })
        .collect();
    assert_eq!(data.len(), 1, "one data item: {data:?}");
    data[0].clone()
}

#[wasm_bindgen_test]
async fn a_socket_already_open_is_adopted() {
    let ws = socket(port().await);
    opened(&ws).await;
    assert_eq!(ws.ready_state(), web_sys::WebSocket::OPEN);
    let t = WsTransport::adopt(ws, "svc").await.expect("an open socket is adopted at once");
    assert!(!t.is_closed());
    assert_eq!(echoed(&t, 1).await, json!({ "echo": { "n": 1 } }));
    assert_eq!(echoed(&t, 2).await, json!({ "echo": { "n": 2 } }), "and carries every call after");
}

#[wasm_bindgen_test]
async fn a_socket_still_connecting_is_adopted_when_it_opens() {
    let ws = socket(port().await);
    assert_eq!(ws.ready_state(), web_sys::WebSocket::CONNECTING);
    let t = WsTransport::adopt(ws, "svc").await.expect("a connecting socket is adopted once it opens");
    assert_eq!(echoed(&t, 3).await, json!({ "echo": { "n": 3 } }));
}

#[wasm_bindgen_test]
async fn a_closed_socket_is_refused() {
    let ws = socket(port().await);
    opened(&ws).await;
    closed(&ws).await;
    let e = WsTransport::adopt(ws, "svc").await.expect_err("a closed socket is refused");
    assert!(e.to_string().contains("closing or closed"), "{e}");
}
