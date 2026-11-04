//! RETURNS-46 review probes, ported: the aggregate's shape through two
//! static children of one type, a dynamic child under a dynamic one, a
//! recursive dynamic child, mutual recursion and `Self` children — each over
//! the service's own routing (`HubTransport`), each event's JSON fixed.

use std::sync::Arc;

use aggregate_probe::mutual::{PingActivation, PingClient, PingEvent, PongEvent};
use aggregate_probe::selfish::{LoopActivation, LoopClient, LoopEvent};
use aggregate_probe::*;
use futures::TryStreamExt;
use plexus_core::plexus::{DynamicHub, HubTransport};

fn service<A: plexus_core::plexus::Activation + plexus_core::plexus::ChildRouter + Clone + 'static>(a: A) -> HubTransport {
    HubTransport::anonymous(Arc::new(DynamicHub::new("svc").register(a)))
}

fn json<E: serde::Serialize>(e: &E) -> String {
    serde_json::to_string(e).unwrap()
}

fn round_trips<E: serde::Serialize + serde::de::DeserializeOwned>(s: &str) {
    let back: E = serde_json::from_str(s).unwrap_or_else(|e| panic!("{e}: {s}"));
    assert_eq!(json(&back), s);
}

#[tokio::test]
async fn a_static_child_of_a_dynamic_one_keeps_the_instances_name() {
    let t = service(RootActivation);
    let root = RootClient::new(&t);
    let evs: Vec<RootEvent> = root.org("acme").stats().tick(2).events().try_collect().await.unwrap();
    match &evs[1] {
        RootEvent::Org { name, event } => {
            assert_eq!(&**name, "acme");
            match &**event {
                OrgEvent::Stats(s) => assert!(matches!(**s, StatsEvent::Tick(N { n: 1 }))),
                other => panic!("{other:?}"),
            }
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(json(&evs[1]), r#"{"method":"org","event":{"name":"acme","event":{"method":"stats","event":{"method":"tick","event":{"n":1}}}}}"#);
}

#[tokio::test]
async fn a_dynamic_child_under_a_dynamic_one_names_both() {
    let t = service(RootActivation);
    let root = RootClient::new(&t);
    let org = root.org("acme");
    let repo = org.repo("r1");
    assert_eq!(repo.path(), "root.acme.r1");
    let evs: Vec<RootEvent> = repo.tick(1).events().try_collect().await.unwrap();
    let s = json(&evs[0]);
    assert_eq!(s, r#"{"method":"org","event":{"name":"acme","event":{"method":"repo","event":{"name":"r1","event":{"method":"tick","event":{"n":0}}}}}}"#);
    round_trips::<RootEvent>(&s);
}

#[tokio::test]
async fn one_type_at_two_paths_is_two_variants() {
    let t = service(RootActivation);
    let root = RootClient::new(&t);
    let l: Vec<RootEvent> = root.left().tick(1).events().try_collect().await.unwrap();
    let r: Vec<RootEvent> = root.right().tick(1).events().try_collect().await.unwrap();
    assert!(matches!(l[0], RootEvent::Left(_)), "{:?}", l[0]);
    assert!(matches!(r[0], RootEvent::Right(_)), "{:?}", r[0]);
    assert_eq!(json(&r[0]), r#"{"method":"right","event":{"method":"tick","event":{"n":0}}}"#);
}

#[tokio::test]
async fn a_recursive_dynamic_child_nests_each_name() {
    let t = service(RootActivation);
    let root = RootClient::new(&t);
    let left = root.left();
    let a = left.dir("a");
    let b = a.sub("b");
    let c = b.sub("c");
    assert_eq!(c.path(), "root.left.a.b.c");
    let evs: Vec<RootEvent> = c.tick(1).events().try_collect().await.unwrap();
    let s = json(&evs[0]);
    assert_eq!(s, r#"{"method":"left","event":{"method":"dir","event":{"name":"a","event":{"method":"sub","event":{"name":"b","event":{"method":"sub","event":{"name":"c","event":{"method":"tick","event":{"n":0}}}}}}}}}"#);
    round_trips::<RootEvent>(&s);
}

#[tokio::test]
async fn mutual_recursion_compiles_and_nests() {
    let t = service(PingActivation);
    let ping = PingClient::new(&t);
    let pong = ping.pong();
    let again = pong.ping("x");
    let evs: Vec<PingEvent> = again.tick(1).events().try_collect().await.unwrap();
    let s = json(&evs[0]);
    assert_eq!(s, r#"{"method":"pong","event":{"method":"ping","event":{"name":"x","event":{"method":"tick","event":{"n":0}}}}}"#);
    round_trips::<PingEvent>(&s);
    let lifted = PingEvent::from(PongEvent::Tick(N { n: 7 }));
    assert_eq!(json(&lifted), r#"{"method":"pong","event":{"method":"tick","event":{"n":7}}}"#);
}

#[tokio::test]
async fn children_written_self_are_the_recursive_case() {
    let t = service(LoopActivation);
    let lp = LoopClient::new(&t);
    let me = lp.me();
    let again = me.again("n");
    assert_eq!(again.path(), "loop.me.n");
    let evs: Vec<LoopEvent> = again.tick(1).events().try_collect().await.unwrap();
    let s = json(&evs[0]);
    assert_eq!(s, r#"{"method":"me","event":{"method":"again","event":{"name":"n","event":{"method":"tick","event":{"n":0}}}}}"#);
    round_trips::<LoopEvent>(&s);
}

#[test]
fn unknown_tags_and_misplaced_names_are_refused() {
    for bad in [
        r#"{"method":"nope","event":{}}"#,
        r#"{"method":"org","event":{"name":"a","event":{"method":"nope","event":{}}}}"#,
        r#"{"method":"org","event":{"event":{"method":"tick","event":{"n":0}}}}"#,
        r#"{"method":"left","event":{"name":"a","event":{"method":"tick","event":{"n":0}}}}"#,
    ] {
        assert!(serde_json::from_str::<RootEvent>(bad).is_err(), "accepted {bad}");
    }
}
