//! MACRO-6 acceptance 1 — menger's `tests/wire.rs`, re-expressed through a
//! **generated client** over [`WsTransport`], against a **real menger** on
//! the real wire.
//!
//! menger's own wire test drives a hand-written jsonrpsee subscription and
//! asserts on `serde_json::Value` (`ev["ticket"]["status"] == "Pending"`).
//! This one asserts on `TicketsEvent::StatusChanged { previous: Status::Pending, .. }`
//! — the same wire, the same server, typed. Everything between the socket
//! and the assertion is MACRO-2's: `WsTransport` → `StreamItem` → `decode`
//! → the activation's own event enum.
//!
//! # Running it
//!
//! The test starts its **own throwaway menger** on ports 35455/35456/35457
//! with a database in `CARGO_TARGET_TMPDIR`, and kills it by pid. It never
//! touches the production instance on 4455–4457.
//!
//! It needs a menger binary, which lives in another repository, so it
//! **skips** (rather than fails) when there is none: set `MENGER_BIN`, or
//! keep a `menger` checkout beside this one with `cargo build` run in it.
//! The skip is loud — it prints why.

use futures::StreamExt;
use menger_tickets_api::{MengerTicketsClient, Status, Ticket, TicketsEvent};
use plexus_client::{ClientEvent, MethodRef, StreamItem, Transport, WsTransport};
use serde_json::json;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// The pinned throwaway ports. Production menger is 4455/4456/4457 and is
/// never touched by this test.
const WS_PORT: u16 = 35455;
const SHELL_PORT: u16 = 35456;
const REST_PORT: u16 = 35457;

const HUB: &str = "menger";

// ---------------------------------------------------------------------------
// The throwaway server
// ---------------------------------------------------------------------------

/// Kills the server on the way out — including when an assertion panics,
/// which is exactly when a leaked server would poison the next run.
struct Throwaway {
    child: Child,
    pid: u32,
}

impl Drop for Throwaway {
    fn drop(&mut self) {
        // By pid, via the handle we own. Never by pattern: a pattern kill
        // would find the production instance.
        let _ = self.child.kill();
        let _ = self.child.wait();
        eprintln!("throwaway menger (pid {}) stopped", self.pid);
    }
}

/// Where the menger binary is, or `None` — in which case the test skips.
fn menger_binary() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("MENGER_BIN") {
        let path = PathBuf::from(explicit);
        return path.is_file().then_some(path);
    }
    // …/plexus-macros-<x>/fixtures/menger-tickets-api → …/menger
    let sibling = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../../menger/target/debug/menger")
        .canonicalize()
        .ok()?;
    sibling.is_file().then_some(sibling)
}

fn port_is_free(port: u16) -> bool {
    std::net::TcpListener::bind(("127.0.0.1", port)).is_ok()
}

async fn start_throwaway(binary: &PathBuf) -> Throwaway {
    for port in [WS_PORT, SHELL_PORT, REST_PORT] {
        assert!(
            port_is_free(port),
            "port {port} is already in use — a previous throwaway menger was \
             not stopped, or something else is listening. Refusing to run \
             against a server this test did not start."
        );
    }

    let db_dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!(
        "macro6-wire-{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir_all(&db_dir).expect("db dir");

    let child = Command::new(binary)
        .arg("--db-path")
        .arg(db_dir.join("t.db"))
        .arg("--port")
        .arg(WS_PORT.to_string())
        .arg("--shell-port")
        .arg(SHELL_PORT.to_string())
        .arg("--rest-port")
        .arg(REST_PORT.to_string())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn menger");

    let pid = child.id();
    eprintln!("throwaway menger (pid {pid}) on ws://127.0.0.1:{WS_PORT}, db {db_dir:?}");
    Throwaway { child, pid }
}

async fn connect() -> WsTransport {
    let url = format!("ws://127.0.0.1:{WS_PORT}");
    for _ in 0..100 {
        if let Ok(transport) = WsTransport::connect(&url, HUB).await {
            return transport;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("throwaway menger never came up on {url}");
}

// ---------------------------------------------------------------------------
// The watch-only mirror — menger `tests/wire.rs::apply`, typed
// ---------------------------------------------------------------------------

/// Every mutation event carries the full affected ticket, so applying one is
/// a single upsert. In `wire.rs` this is a `match ev["type"].as_str()` with
/// a `panic!` arm for anything unrecognised; here it is a `match` over the
/// event enum, so an unhandled variant is a compile error.
fn apply(mirror: &mut HashMap<String, Ticket>, event: &TicketsEvent) -> &'static str {
    match event {
        TicketsEvent::Created { ticket } => {
            mirror.insert(ticket.id.clone(), ticket.clone());
            "created"
        }
        TicketsEvent::StatusChanged { ticket, .. } => {
            mirror.insert(ticket.id.clone(), ticket.clone());
            "status_changed"
        }
        TicketsEvent::UnlocksChanged { ticket, .. } => {
            mirror.insert(ticket.id.clone(), ticket.clone());
            "unlocks_changed"
        }
        TicketsEvent::ChildrenChanged { ticket, .. } => {
            mirror.insert(ticket.id.clone(), ticket.clone());
            "children_changed"
        }
        other => panic!("unexpected watch event {other:?}"),
    }
}

/// The one ticket a single-event call answers with.
fn one(events: Vec<TicketsEvent>) -> TicketsEvent {
    assert_eq!(events.len(), 1, "expected exactly one event, got {events:?}");
    events.into_iter().next().unwrap()
}

fn created(events: Vec<TicketsEvent>) -> Ticket {
    match one(events) {
        TicketsEvent::Created { ticket } => ticket,
        other => panic!("expected `created`, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// The test
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn menger_wire_assertions_through_a_generated_client_over_websocket() {
    let Some(binary) = menger_binary() else {
        eprintln!(
            "SKIPPED: no menger binary. Set MENGER_BIN, or run `cargo build` \
             in a menger checkout beside this repository."
        );
        return;
    };

    let _server = start_throwaway(&binary).await;
    let transport = connect().await;
    let client = MengerTicketsClient::new(&transport);

    // -- health -------------------------------------------------------------
    // wire.rs: ev["type"] == "health", ev["ticket_count"] == 0.
    match one(client.health_all().await.expect("health")) {
        TicketsEvent::Health {
            ticket_count,
            version,
            ..
        } => {
            assert_eq!(ticket_count, 0, "a fresh database");
            assert!(!version.is_empty());
        }
        other => panic!("expected `health`, got {other:?}"),
    }

    // -- the full envelope --------------------------------------------------
    // The RPC-level client, on the same connection: this transport is the
    // one that carries `StreamMetadata`, and nothing else proves it.
    let mut raw = transport.call(MethodRef::new("tickets", "health"), json!({}));
    let item = raw.next().await.expect("an item").expect("no transport error");
    match &item {
        StreamItem::Data {
            metadata,
            content_type,
            ..
        } => {
            let metadata = metadata.as_ref().expect("WebSocket carries metadata");
            assert!(
                metadata.provenance.contains(&"tickets".to_string()),
                "provenance names the activation: {:?}",
                metadata.provenance
            );
            assert!(!metadata.plexus_hash.is_empty(), "and the config hash");
            assert!(metadata.timestamp > 0, "and when it was wrapped");
            eprintln!("full envelope: metadata={metadata:?} content_type={content_type:?}");
        }
        other => panic!("expected a data item, got {other:?}"),
    }
    drop(raw);

    // -- watch, subscribed before any mutation ------------------------------
    // The subscribe frame is written by `call`, not by the first poll, so
    // this really is subscribed here. The round trip below flushes it past
    // the server's dispatcher before the first mutation is sent.
    let mut watch = client.watch(None);
    let _ = client.health_all().await.expect("round trip");
    tokio::time::sleep(Duration::from_millis(250)).await;

    // -- create -------------------------------------------------------------
    // wire.rs: ev["type"] == "created", id "WIRE-1", status "Pending".
    let ticket = created(
        client
            .create_all(
                "WIRE".into(),
                "first over the wire".into(),
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .expect("create"),
    );
    assert_eq!(ticket.id, "WIRE-1");
    assert_eq!(ticket.status, Status::Pending);
    assert_eq!(ticket.seq, 1);
    assert!(ticket.parent.is_none(), "born parentless");
    assert!(ticket.children.is_empty(), "and childless");

    // -- set_status ---------------------------------------------------------
    // wire.rs: ev["type"] == "status_changed", ev["previous"] == "Pending".
    match one(
        client
            .set_status_all("WIRE-1".into(), "Ready".into())
            .await
            .expect("set_status"),
    ) {
        TicketsEvent::StatusChanged { ticket, previous } => {
            assert_eq!(previous, Status::Pending);
            assert_eq!(ticket.status, Status::Ready);
        }
        other => panic!("expected `status_changed`, got {other:?}"),
    }

    // -- list replays current state ----------------------------------------
    // wire.rs: ev["type"] == "ticket", id "WIRE-1".
    match one(
        client
            .list_all(Some("WIRE".into()), None, None)
            .await
            .expect("list"),
    ) {
        TicketsEvent::Ticket { ticket } => assert_eq!(ticket.id, "WIRE-1"),
        other => panic!("expected `ticket`, got {other:?}"),
    }

    // -- a dangling blocker -------------------------------------------------
    // wire.rs::dangling_blocker_over_the_wire: a Ready ticket with a blocker
    // that names nothing stays out of the ready set, and `blockers` marks
    // the id as dangling — its own event category, not prose.
    created(
        client
            .create_all(
                "DG".into(),
                "C".into(),
                None,
                None,
                Some(vec!["DG-99".into()]),
                None,
                None,
            )
            .await
            .expect("create DG-1"),
    );
    client
        .set_status_all("DG-1".into(), "Ready".into())
        .await
        .expect("promote DG-1");

    let ready = client
        .ready_set_all(Some("DG".into()))
        .await
        .expect("ready_set");
    assert!(ready.is_empty(), "DG-1 must not be ready: {ready:?}");

    match one(client.blockers_all("DG-1".into()).await.expect("blockers")) {
        TicketsEvent::DanglingBlocker { id } => assert_eq!(id, "DG-99"),
        other => panic!("expected `dangling_blocker`, got {other:?}"),
    }

    // -- parent / children --------------------------------------------------
    // wire.rs::parent_children_and_the_parent_filter_over_the_wire.
    let parent = created(
        client
            .create_all("PA".into(), "P".into(), None, None, None, None, None)
            .await
            .expect("create PA-1"),
    );
    assert_eq!(parent.id, "PA-1");

    let child = created(
        client
            .create_all(
                "PA".into(),
                "C".into(),
                Some("execution".into()),
                None,
                None,
                None,
                Some("PA-1".into()),
            )
            .await
            .expect("create PA-2"),
    );
    assert_eq!(child.id, "PA-2");
    assert_eq!(child.parent.as_deref(), Some("PA-1"));

    // Both ends agree when asked directly.
    match one(client.get_all("PA-1".into()).await.expect("get PA-1")) {
        TicketsEvent::Ticket { ticket } => assert_eq!(ticket.children, vec!["PA-2".to_string()]),
        other => panic!("expected `ticket`, got {other:?}"),
    }
    match one(client.get_all("PA-2".into()).await.expect("get PA-2")) {
        TicketsEvent::Ticket { ticket } => assert_eq!(ticket.parent.as_deref(), Some("PA-1")),
        other => panic!("expected `ticket`, got {other:?}"),
    }

    // wire.rs acceptance 4: `list {parent}` is exactly that ticket's children.
    let children: Vec<String> = client
        .list_all(None, None, Some("PA-1".into()))
        .await
        .expect("list by parent")
        .into_iter()
        .map(|e| match e {
            TicketsEvent::Ticket { ticket } => ticket.id,
            other => panic!("expected `ticket`, got {other:?}"),
        })
        .collect();
    assert_eq!(children, vec!["PA-2".to_string()]);

    // -- an unlocks edge ----------------------------------------------------
    // The fourth watch event kind: a newly created ticket naming an existing
    // one in `blocked_by` updates that one's `unlocks`.
    created(
        client
            .create_all("RS".into(), "A".into(), None, None, None, None, None)
            .await
            .expect("create RS-1"),
    );
    created(
        client
            .create_all(
                "RS".into(),
                "B".into(),
                None,
                None,
                Some(vec!["RS-1".into()]),
                None,
                None,
            )
            .await
            .expect("create RS-2"),
    );

    // -- the watch-only mirror ---------------------------------------------
    // wire.rs asserts a watch-only client stays coherent. Same claim, over
    // all four mutation event kinds, decoded into the activation's types.
    let mut mirror: HashMap<String, Ticket> = HashMap::new();
    let mut kinds: Vec<&'static str> = Vec::new();
    let mut first_transition: Option<Status> = None;

    loop {
        match tokio::time::timeout(Duration::from_millis(1500), watch.next()).await {
            Err(_) => break, // the stream went quiet: every change has arrived
            Ok(None) => panic!("the watch stream ended"),
            Ok(Some(event)) => match event.expect("no client error on watch") {
                ClientEvent::Data(event) => {
                    if let TicketsEvent::StatusChanged { previous, .. } = &event {
                        first_transition.get_or_insert(*previous);
                    }
                    kinds.push(apply(&mut mirror, &event));
                }
                ClientEvent::Progress { .. } => {}
            },
        }
    }

    eprintln!("watch delivered {} events: {kinds:?}", kinds.len());
    for expected in ["created", "status_changed", "unlocks_changed", "children_changed"] {
        assert!(
            kinds.contains(&expected),
            "the watch mirror saw no `{expected}`: {kinds:?}"
        );
    }
    assert_eq!(
        first_transition,
        Some(Status::Pending),
        "the first transition was out of Pending"
    );

    assert_eq!(mirror["WIRE-1"].status, Status::Ready, "the watcher saw the promotion");
    assert_eq!(
        mirror["PA-1"].children,
        vec!["PA-2".to_string()],
        "children_changed carried the full parent"
    );
    assert_eq!(
        mirror["RS-1"].unlocks,
        vec!["RS-2".to_string()],
        "unlocks_changed carried the full ticket"
    );
    assert_eq!(mirror["DG-1"].blocked_by, vec!["DG-99".to_string()]);

    // Server and watch-only mirror agree on the ready set, computed two ways.
    let server_ready: Vec<String> = client
        .ready_set_all(Some("WIRE".into()))
        .await
        .expect("ready_set")
        .into_iter()
        .map(|e| match e {
            TicketsEvent::Ticket { ticket } => ticket.id,
            other => panic!("expected `ticket`, got {other:?}"),
        })
        .collect();
    let local_ready: Vec<String> = {
        let mut ids: Vec<String> = mirror
            .values()
            .filter(|t| t.epic == "WIRE" && t.status == Status::Ready)
            .filter(|t| {
                t.blocked_by.iter().all(|b| {
                    mirror.get(b).is_some_and(|blocker| {
                        matches!(blocker.status, Status::Complete | Status::Superseded)
                    })
                })
            })
            .map(|t| t.id.clone())
            .collect();
        ids.sort();
        ids
    };
    assert_eq!(server_ready, vec!["WIRE-1".to_string()]);
    assert_eq!(server_ready, local_ready, "watch-derivable, as wire.rs claims");

    // -- unsubscribe --------------------------------------------------------
    // Dropping the stream sends `menger.call_unsub`. The observable claim
    // here is that the connection survives it and keeps serving — the frame
    // itself is pinned against a scripted server in `ws_transport.rs`, where
    // the bytes can be read.
    drop(watch);
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(!transport.is_closed(), "the unsubscribe did not disturb the socket");
    match one(client.health_all().await.expect("health after unsubscribe")) {
        TicketsEvent::Health { ticket_count, .. } => assert_eq!(ticket_count, 6),
        other => panic!("expected `health`, got {other:?}"),
    }
}
