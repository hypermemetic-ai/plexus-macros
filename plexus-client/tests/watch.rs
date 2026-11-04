//! RETURNS-38 · `watch((a, b, c))` — several method streams in, one out,
//! whose item is exactly the union of theirs.
//!
//! The behaviour table, pinned: the exact type (an exhaustive `match` with no
//! wildcard), arrival order, the end, an error as an item, `Errors::Stop`,
//! and every held stream dropped exactly once when the merge is — which, on
//! a `WsTransport`, is one `call_unsub` each (the wire half is
//! `fixtures/tickets-api/tests/watch_wire.rs`).

use futures::channel::mpsc;
use futures::executor::block_on;
use futures::stream::{self, Stream, StreamExt};
use futures::FutureExt;
use plexus_client::{
    decode, watch, watch_with, ClientError, Errors, MemoryTransport, MethodRef, StreamItem,
    Transport, Watched12, Watched2, Watched3,
};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};

// Three methods, three event types — nothing in common but the merge.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TicketEv {
    id: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ChatEv {
    Posted { message: u32 },
    Read { upto: u32 },
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct DeployEv(String);

const TICKETS_WATCH: MethodRef = MethodRef::new("tickets", "watch");
const CHATS_WATCH: MethodRef = MethodRef::new("chats", "watch");
const DEPLOYMENTS_WATCH: MethodRef = MethodRef::new("deployments", "watch");

type Item<E> = Result<E, ClientError>;

#[test]
fn exact_union_match_without_wildcard() {
    let t = MemoryTransport::new()
        .script_events(TICKETS_WATCH, &[TicketEv { id: "MNG-1".into() }])
        .script_events(CHATS_WATCH, &[ChatEv::Posted { message: 7 }, ChatEv::Read { upto: 7 }])
        .script_events(DEPLOYMENTS_WATCH, &[DeployEv("R1".into())]);

    let s = watch((
        decode::<TicketEv>(t.call(TICKETS_WATCH, json!({}))),
        decode::<ChatEv>(t.call(CHATS_WATCH, json!({}))),
        decode::<DeployEv>(t.call(DEPLOYMENTS_WATCH, json!({}))),
    ));
    // The item type is written out: exactly the three, in order, never
    // widened to anything they share.
    let items: Vec<Watched3<Item<TicketEv>, Item<ChatEv>, Item<DeployEv>>> = block_on(s.collect());
    assert_eq!(items.len(), 4);

    let (mut tickets, mut posted, mut read, mut deploys) = (0, 0, 0, 0);
    for it in items {
        // No `_` arm: a fourth source, or a new ChatEv variant, fails this
        // compile rather than falling through.
        match it {
            Watched3::A(Ok(TicketEv { id })) => {
                assert_eq!(id, "MNG-1");
                tickets += 1
            }
            Watched3::B(Ok(ChatEv::Posted { message })) => {
                assert_eq!(message, 7);
                posted += 1
            }
            Watched3::B(Ok(ChatEv::Read { .. })) => read += 1,
            Watched3::C(Ok(DeployEv(r))) => {
                assert_eq!(r, "R1");
                deploys += 1
            }
            Watched3::A(Err(e)) | Watched3::B(Err(e)) | Watched3::C(Err(e)) => panic!("{e}"),
        }
    }
    assert_eq!((tickets, posted, read, deploys), (1, 1, 1, 1));
    // Every call went out once.
    assert_eq!(t.calls().len(), 3);
}

#[test]
fn one_argument_is_the_stream_itself() {
    // `(A,)` is A: its type unchanged, no enum around it.
    let s: stream::Iter<std::vec::IntoIter<u8>> = watch((stream::iter(vec![1u8, 2]),));
    assert_eq!(block_on(s.collect::<Vec<_>>()), vec![1, 2]);
}

#[test]
fn arity_twelve_builds_and_runs() {
    let one = |n: u8| stream::iter(vec![n]);
    let s = watch((one(0), one(1), one(2), one(3), one(4), one(5), one(6), one(7), one(8), one(9), one(10), one(11)));
    let mut seen: Vec<usize> = block_on(s.map(|w: Watched12<u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8, u8>| {
        // the source's position agrees with what it carried
        let (index, n) = match w {
            Watched12::A(n) => (0, n),
            Watched12::B(n) => (1, n),
            Watched12::C(n) => (2, n),
            Watched12::D(n) => (3, n),
            Watched12::E(n) => (4, n),
            Watched12::F(n) => (5, n),
            Watched12::G(n) => (6, n),
            Watched12::H(n) => (7, n),
            Watched12::I(n) => (8, n),
            Watched12::J(n) => (9, n),
            Watched12::K(n) => (10, n),
            Watched12::L(n) => (11, n),
        };
        assert_eq!(index, n as usize);
        assert_eq!(w.index(), index);
        index
    })
    .collect());
    seen.sort();
    assert_eq!(seen, (0..12).collect::<Vec<_>>());
}

#[test]
fn arrival_order() {
    let (ta, ra) = mpsc::unbounded::<u8>();
    let (tb, rb) = mpsc::unbounded::<&'static str>();
    let (tc, rc) = mpsc::unbounded::<char>();
    let mut s = watch((ra, rb, rc));

    // Nothing has arrived: the merge waits.
    assert!(s.next().now_or_never().is_none());

    tb.unbounded_send("b1").unwrap();
    assert_eq!(block_on(s.next()), Some(Watched3::B("b1")));
    tc.unbounded_send('c').unwrap();
    assert_eq!(block_on(s.next()), Some(Watched3::C('c')));
    ta.unbounded_send(1).unwrap();
    assert_eq!(block_on(s.next()), Some(Watched3::A(1)));
    tb.unbounded_send("b2").unwrap();
    assert_eq!(block_on(s.next()), Some(Watched3::B("b2")));
    assert!(s.next().now_or_never().is_none());
}

#[test]
fn ends_when_all_end() {
    let (tb, rb) = mpsc::unbounded::<u8>();
    let mut s = watch((stream::iter(vec!["a"]), rb));

    assert_eq!(block_on(s.next()), Some(Watched2::A("a")));
    // A has ended; B is open, so the merge goes on.
    assert!(s.next().now_or_never().is_none());
    tb.unbounded_send(9).unwrap();
    assert_eq!(block_on(s.next()), Some(Watched2::B(9)));
    drop(tb);
    assert_eq!(block_on(s.next()), None);
    // Fused: it stays ended.
    assert_eq!(block_on(s.next()), None);
}

#[test]
fn error_item_others_go_on() {
    // A client stream ends after its error (decode's rule); the merge does
    // not.
    let t = MemoryTransport::new()
        .script(
            TICKETS_WATCH,
            vec![StreamItem::Error {
                metadata: None,
                recoverable: false,
                message: Some("forbidden".into()),
                code: Some("-32003".into()),
                error: plexus_client::RemoteError::Forbidden {
                    reason: plexus_client::DenyReason::NotAccepted,
                },
                fields: Default::default(),
            }],
        )
        .script_events(CHATS_WATCH, &[ChatEv::Posted { message: 1 }, ChatEv::Posted { message: 2 }]);
    let s = watch((
        decode::<TicketEv>(t.call(TICKETS_WATCH, json!({}))),
        decode::<ChatEv>(t.call(CHATS_WATCH, json!({}))),
    ));
    let items = block_on(s.collect::<Vec<_>>());
    assert_eq!(items.len(), 3, "{items:?}");
    let errors: Vec<_> = items
        .iter()
        .filter_map(|w| match w {
            Watched2::A(Err(e)) => Some(e.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(errors.len(), 1);
    assert!(errors[0].is_remote(), "the error names its source by its variant: {:?}", errors[0]);
    let posted = items
        .iter()
        .filter(|w| matches!(w, Watched2::B(Ok(ChatEv::Posted { .. }))))
        .count();
    assert_eq!(posted, 2);
}

/// A stream that counts how many times it is dropped — on a `WsTransport`
/// that drop is the `call_unsub`.
struct Counted<S> {
    inner: S,
    drops: Arc<AtomicUsize>,
}
impl<S: Stream + Unpin> Stream for Counted<S> {
    type Item = S::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<S::Item>> {
        Pin::new(&mut self.inner).poll_next(cx)
    }
}
impl<S> Drop for Counted<S> {
    fn drop(&mut self) {
        self.drops.fetch_add(1, Ordering::SeqCst);
    }
}
fn counted<S>(inner: S, drops: &Arc<AtomicUsize>) -> Counted<S> {
    Counted { inner, drops: drops.clone() }
}

#[test]
fn stop_on_error_ends() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (_tb, rb) = mpsc::unbounded::<Result<u8, String>>(); // B never ends
    let (_tc, rc) = mpsc::unbounded::<Result<char, String>>(); // nor C
    let mut s = watch_with(
        (
            counted(stream::iter(vec![Err::<&str, String>("refused".into())]), &drops),
            counted(rb, &drops),
            counted(rc, &drops),
        ),
        Errors::Stop,
    );
    match block_on(s.next()) {
        Some(Watched3::A(Err(e))) => assert_eq!(e, "refused"),
        other => panic!("{other:?}"),
    }
    // The failure was the last item, and every held stream went with it —
    // B and C unsubscribed at once, not when the merge is dropped.
    assert_eq!(drops.load(Ordering::SeqCst), 3);
    assert_eq!(block_on(s.next()), None);
    drop(s);
    assert_eq!(drops.load(Ordering::SeqCst), 3, "dropped once each, never twice");
}

#[test]
fn errors_continue_is_the_default() {
    let s = watch_with(
        (stream::iter(vec![Err::<u8, &str>("x"), Ok(1)]), stream::iter(vec![Ok::<u8, &str>(2)])),
        Errors::Continue,
    );
    assert_eq!(block_on(s.collect::<Vec<_>>()).len(), 3);
}

#[test]
fn drop_drops_each_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    let (_ta, ra) = mpsc::unbounded::<u8>();
    let (_tb, rb) = mpsc::unbounded::<u8>();
    let (_tc, rc) = mpsc::unbounded::<u8>();
    let mut s = watch((counted(ra, &drops), counted(rb, &drops), counted(rc, &drops)));
    assert!(s.next().now_or_never().is_none()); // all three live and parked
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    drop(s);
    assert_eq!(drops.load(Ordering::SeqCst), 3);
}

#[test]
fn an_ended_stream_is_let_go_at_once() {
    // A finished subscription holds nothing while the others go on.
    let drops = Arc::new(AtomicUsize::new(0));
    let (_tb, rb) = mpsc::unbounded::<u8>();
    let mut s = watch((counted(stream::iter(vec![1u8]), &drops), counted(rb, &drops)));
    assert_eq!(block_on(s.next()), Some(Watched2::A(1)));
    assert!(s.next().now_or_never().is_none());
    assert_eq!(drops.load(Ordering::SeqCst), 1);
    drop(s);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
}

/// Perf (RETURNS-38's `perf` field): the merge's cost per item over a bare
/// stream, 100,000 items. `cargo test -p plexus-client --release --test
/// watch -- --ignored --nocapture perf`.
#[test]
#[ignore]
fn perf_per_item_overhead() {
    use std::time::Instant;
    const N: usize = 100_000;
    let bare = || stream::iter(0..N as u64);
    let runs = 7;
    let mut bare_ns = Vec::new();
    let mut merged_ns = Vec::new();
    for _ in 0..runs {
        let t0 = Instant::now();
        let n = block_on(bare().fold(0u64, |a, x| async move { a.wrapping_add(x) }));
        bare_ns.push(t0.elapsed().as_nanos() as f64);
        std::hint::black_box(n);

        // Three sources sharing the N items: the merge is on every one.
        let third = (N / 3) as u64;
        let t0 = Instant::now();
        let n = block_on(
            watch((stream::iter(0..third), stream::iter(third..2 * third), stream::iter(2 * third..N as u64)))
                .fold(0u64, |a, w| async move {
                    a.wrapping_add(match w {
                        Watched3::A(x) | Watched3::B(x) | Watched3::C(x) => x,
                    })
                }),
        );
        merged_ns.push(t0.elapsed().as_nanos() as f64);
        std::hint::black_box(n);
    }
    bare_ns.sort_by(|a, b| a.partial_cmp(b).unwrap());
    merged_ns.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let (b, m) = (bare_ns[runs / 2] / N as f64, merged_ns[runs / 2] / N as f64);
    println!("perf rust: bare={b:.2} ns/item merged={m:.2} ns/item overhead={:.2} ns/item", m - b);
    println!(
        "memory rust: merge of 3 boxed streams = {} bytes inline ({} per held subscription beyond the stream itself)",
        std::mem::size_of::<plexus_client::Watching3<plexus_client::MethodStream<'static, u8>, plexus_client::MethodStream<'static, u8>, plexus_client::MethodStream<'static, u8>>>(),
        (std::mem::size_of::<plexus_client::Watching3<plexus_client::MethodStream<'static, u8>, plexus_client::MethodStream<'static, u8>, plexus_client::MethodStream<'static, u8>>>()
            - 3 * std::mem::size_of::<plexus_client::MethodStream<'static, u8>>())
            / 3
    );
    assert!(m - b < 1000.0, "≤ 1 µs per item: {:.2} ns", m - b);
}
