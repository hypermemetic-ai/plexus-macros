//! RETURNS-46: an activation's aggregate nests its children's, static and
//! dynamic, and a child's call lifts into any ancestor's.
//!
//! The tree (`src/lib.rs`) is served by a `DynamicHub` and called through
//! plexus-core's `HubTransport` — the hub's own routing, so a child's method
//! goes to `tree.branch.grow`, a dynamic instance's to `tree.a.watch`, as
//! over the wire. What the calls decode to is compared with `expected/`:
//!
//! - every level's `aggregate.<level>.jsonl` round-trips through that
//!   level's aggregate byte for byte;
//! - the calls of `calls.json`, made through the root's client, yield the
//!   root's aggregate — `aggregate.tree.jsonl` — and send exactly
//!   `calls.json`'s paths;
//! - `select_all` over three levels' `.events()` and two dynamic instances'
//!   is one `TreeEvent` stream, matched with no wildcard;
//! - a dynamic child's instances each say their name;
//! - a child's call made on the child's own client lifts with `From`.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use aggregate_tree::{
    BranchClient, BranchEvent, Bud, Fell, FoliageEvent, GrowError, ItemClient, ItemEvent, LeafClient, Pong, Seen,
    Tick, TreeActivation, TreeClient, TreeEvent,
};
use futures::{StreamExt, TryStreamExt};
use plexus_client::{ItemStream, MethodRef, Transport};
use plexus_core::plexus::{DynamicHub, HubTransport};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

// ------------------------------------------------------------------ fixture files

fn read(rel: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("expected").join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn lines(rel: &str) -> Vec<String> {
    read(rel).lines().filter(|l| !l.trim().is_empty()).map(str::to_owned).collect()
}

fn line<E: Serialize>(e: &E) -> String {
    serde_json::to_string(e).expect("an aggregate serializes")
}

// ------------------------------------------------------------------ the service

/// The hub's transport, recording each call's path and params.
#[derive(Clone)]
struct Recording {
    inner: HubTransport,
    calls: Arc<Mutex<Vec<Value>>>,
}

impl Recording {
    fn new() -> Self {
        let hub = Arc::new(DynamicHub::new("tree_service").register(TreeActivation));
        Self { inner: HubTransport::anonymous(hub), calls: Arc::default() }
    }

    fn calls(&self) -> Vec<Value> {
        self.calls.lock().unwrap().clone()
    }
}

impl Transport for Recording {
    fn call(&self, method: MethodRef, params: Value) -> ItemStream<'_> {
        self.calls.lock().unwrap().push(json!({"path": method.path(), "method": method.method(), "params": params}));
        self.inner.call(method, params)
    }
}

// ------------------------------------------------------------------ 1. the JSON, per level

fn round_trips<E: Serialize + DeserializeOwned>(rel: &str) {
    for l in lines(rel) {
        let e: E = serde_json::from_str(&l).unwrap_or_else(|err| panic!("{rel}: {err}: {l}"));
        assert_eq!(line(&e), l, "{rel}: re-serialized");
    }
}

#[test]
fn every_levels_json_round_trips_through_its_aggregate() {
    round_trips::<TreeEvent>("aggregate.tree.jsonl");
    round_trips::<BranchEvent>("aggregate.tree.branch.jsonl");
    round_trips::<FoliageEvent>("aggregate.tree.branch.leaf.jsonl");
    round_trips::<ItemEvent>("aggregate.tree.item.jsonl");
}

#[test]
fn a_wrong_nesting_is_refused() {
    for wrong in [
        // a grandchild's event straight under the root
        json!({"method": "fall", "event": {"leaf": 1}}),
        // the leaf one level too high
        json!({"method": "leaf", "event": {"method": "fall", "event": {"leaf": 1}}}),
        // a static child's event under the dynamic child's tag, with no name
        json!({"method": "item", "event": {"method": "watch", "event": {"seen": "a", "n": 1}}}),
        // a dynamic instance's event without its name
        json!({"method": "item", "event": {"event": {"method": "watch", "event": {"seen": "a", "n": 1}}}}),
        // a method of the branch under the leaf
        json!({"method": "branch", "event": {"method": "leaf", "event": {"method": "grow", "event": {"ok": {"bud": 1}}}}}),
    ] {
        assert!(serde_json::from_value::<TreeEvent>(wrong.clone()).is_err(), "accepted {wrong}");
    }
}

// ------------------------------------------------------------------ 2. through the root's client

#[tokio::test]
async fn the_roots_client_yields_the_roots_aggregate_for_the_whole_tree() {
    let t = Recording::new();
    let tree = TreeClient::new(&t);
    let mut got: Vec<TreeEvent> = vec![TreeEvent::Ping(tree.ping().await.expect("ping"))];
    got.extend(tree.count(2).events().try_collect::<Vec<_>>().await.expect("count"));
    let branch = tree.branch();
    got.extend(branch.grow(3).events().try_collect::<Vec<_>>().await.expect("grow"));
    let leaf = branch.leaf();
    got.extend(leaf.fall(2).events().try_collect::<Vec<_>>().await.expect("fall"));
    got.extend(tree.item("a").watch(2).events().try_collect::<Vec<_>>().await.expect("a"));
    got.extend(tree.item("b").watch(1).events().try_collect::<Vec<_>>().await.expect("b"));

    assert_eq!(got.iter().map(line).collect::<Vec<_>>(), lines("aggregate.tree.jsonl"));
    let calls: Value = serde_json::from_str(&read("calls.json")).unwrap();
    assert_eq!(Value::Array(t.calls()), calls, "the paths and params sent");
}

#[tokio::test]
async fn each_levels_own_client_yields_its_own_aggregate() {
    let t = Recording::new();
    let branch = BranchClient::at(&t, "tree.branch");
    let mut got: Vec<BranchEvent> = branch.grow(3).events().try_collect().await.unwrap();
    got.extend(branch.leaf().fall(2).events().try_collect::<Vec<_>>().await.unwrap());
    assert_eq!(got.iter().map(line).collect::<Vec<_>>(), lines("aggregate.tree.branch.jsonl"));

    let leaf: Vec<FoliageEvent> = LeafClient::at(&t, "tree.branch.leaf").fall(2).events().try_collect().await.unwrap();
    assert_eq!(leaf.iter().map(line).collect::<Vec<_>>(), lines("aggregate.tree.branch.leaf.jsonl"));

    let mut items: Vec<ItemEvent> = ItemClient::at(&t, "tree.a").watch(2).events().try_collect().await.unwrap();
    items.extend(ItemClient::at(&t, "tree.b").watch(1).events().try_collect::<Vec<_>>().await.unwrap());
    assert_eq!(items.iter().map(line).collect::<Vec<_>>(), lines("aggregate.tree.item.jsonl"));
}

// ------------------------------------------------------------------ 3. one listener, three levels

#[tokio::test]
async fn select_all_over_three_levels_is_matched_with_no_wildcard() {
    let t = Recording::new();
    let tree = TreeClient::new(&t);
    let branch = tree.branch();
    let leaf = branch.leaf();
    let (a, b) = (tree.item("a"), tree.item("b"));
    let mut all = futures::stream::select_all([
        tree.count(2).events(),
        branch.grow(3).events(),
        leaf.fall(2).events(),
        a.watch(2).events(),
        b.watch(1).events(),
    ]);

    let (mut ticks, mut buds, mut frosts, mut leaves, mut seen) = (0, 0, 0, 0, Vec::new());
    while let Some(ev) = all.next().await {
        // Exhaustive: every variant of every level, and no `_`.
        match ev.expect("no call fails") {
            TreeEvent::Ping(Pong { .. }) => unreachable!("ping is a one-item method: no .events()"),
            TreeEvent::Count(Tick { n }) => ticks += n,
            TreeEvent::Branch(BranchEvent::Grow(Ok(Bud { bud }))) => buds += bud,
            TreeEvent::Branch(BranchEvent::Grow(Err(GrowError::Frost { at }))) => frosts += at,
            TreeEvent::Branch(BranchEvent::Leaf(FoliageEvent::Fall(Fell { leaf }))) => leaves += leaf,
            TreeEvent::Item { name, event: ItemEvent::Watch(Seen { seen: s, n }) } => {
                assert_eq!(name, s, "the event names the instance it came from");
                seen.push(format!("{name}{n}"));
            }
        }
    }
    assert_eq!((ticks, buds, frosts, leaves), (1 + 2, 1 + 3, 2, 1 + 2));
    seen.sort();
    assert_eq!(seen, ["a1", "a2", "b1"]);
}

#[tokio::test]
async fn a_dynamic_childs_instances_arrive_with_their_names() {
    let t = Recording::new();
    let tree = TreeClient::new(&t);
    let (a, b) = (tree.item("a"), tree.item("b"));
    assert_eq!((a.path(), b.path()), ("tree.a", "tree.b"));
    let merged: Vec<TreeEvent> =
        futures::stream::select_all([a.watch(2).events(), b.watch(2).events()]).try_collect().await.unwrap();
    let mut names: Vec<(String, u32)> = merged
        .into_iter()
        .map(|ev| match ev {
            TreeEvent::Item { name, event: ItemEvent::Watch(s) } => {
                assert_eq!(name, s.seen);
                (name, s.n)
            }
            other => panic!("not an item's event: {other:?}"),
        })
        .collect();
    names.sort();
    assert_eq!(names, [("a".into(), 1), ("a".into(), 2), ("b".into(), 1), ("b".into(), 2)]);
    let paths: Vec<Value> = t.calls().iter().map(|c| c["path"].clone()).collect();
    assert_eq!(paths, [json!("tree.a"), json!("tree.b")]);
}

#[tokio::test]
async fn an_unknown_instance_is_the_services_answer() {
    let t = Recording::new();
    let err = TreeClient::new(&t).item("zz").watch(1).events().try_collect::<Vec<_>>().await.unwrap_err();
    assert!(err.to_string().contains("zz"), "{err}");
}

// ------------------------------------------------------------------ 4. From, one level at a time

#[tokio::test]
async fn a_childs_own_aggregate_lifts_into_its_ancestors_by_from() {
    let t = Recording::new();
    let grown: Vec<TreeEvent> =
        BranchClient::at(&t, "tree.branch").grow(1).events().map_ok(TreeEvent::from).try_collect().await.unwrap();
    assert_eq!(line(&grown[0]), r#"{"method":"branch","event":{"method":"grow","event":{"ok":{"bud":1}}}}"#);

    let fell: Vec<TreeEvent> = LeafClient::at(&t, "tree.branch.leaf")
        .fall(1)
        .events()
        .map_ok(|e| TreeEvent::from(BranchEvent::from(e)))
        .try_collect()
        .await
        .unwrap();
    assert_eq!(line(&fell[0]), lines("aggregate.tree.jsonl")[6]);
}
