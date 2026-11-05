//! RETURNS-46 review: an instance name that is empty, holds a `.`, is not a
//! segment of the service's path grammar, or is a static sibling's segment
//! cannot reach the instance it names. The accessor stays
//! infallible; every call through such an instance — or anything below it —
//! fails at once with `ClientError::InvalidName`, and nothing is sent.

use std::sync::{Arc, Mutex};

use aggregate_probe::*;
use futures::TryStreamExt;
use plexus_client::{ClientError, ItemStream, MethodRef, NameRefusal, Transport};
use serde_json::Value;

#[derive(Default, Clone)]
struct Recording(Arc<Mutex<Vec<String>>>);

impl Transport for Recording {
    fn call(&self, method: MethodRef, _params: Value) -> ItemStream<'_> {
        self.0.lock().unwrap().push(method.qualified());
        Fast(1).call(method, Value::Null)
    }
}

fn refused(r: Result<Vec<RootEvent>, ClientError>, name: &str, reason: NameRefusal) {
    match r {
        Err(ClientError::InvalidName { name: n, reason: why }) => {
            assert_eq!(n, name);
            assert_eq!(why, reason, "{name:?}");
        }
        other => panic!("{name:?}: {other:?}"),
    }
}

#[tokio::test]
async fn a_dotted_name_is_refused_and_nothing_is_sent() {
    let t = Recording::default();
    let root = RootClient::new(&t);
    let org = root.org("acme.stats");
    refused(org.tick(1).events().try_collect().await, "acme.stats", NameRefusal::Dotted);
    // below it too: `root.acme.stats` would be the org `acme`'s static child
    refused(org.stats().tick(1).events().try_collect().await, "acme.stats", NameRefusal::Dotted);
    assert!(t.0.lock().unwrap().is_empty(), "sent {:?}", t.0.lock().unwrap());
}

#[tokio::test]
async fn a_static_siblings_segment_is_refused() {
    let t = Recording::default();
    let root = RootClient::new(&t);
    refused(root.org("left").tick(1).events().try_collect().await, "left", NameRefusal::StaticChild);
    refused(root.org("right").tick(1).events().try_collect().await, "right", NameRefusal::StaticChild);
    assert!(t.0.lock().unwrap().is_empty());
}

/// The service splits a path at its first dot and takes that segment as a
/// child, never as the parent's method: an instance named after one of the
/// parent's methods is the instance (the re-review's `root.tick.tick`).
#[tokio::test]
async fn an_instance_named_after_a_method_of_the_parent_is_reached() {
    use plexus_core::plexus::{DynamicHub, HubTransport};
    let t = HubTransport::anonymous(Arc::new(DynamicHub::new("svc").register(RootActivation)));
    let root = RootClient::new(&t);
    for name in ["tick", "schema"] {
        let evs: Vec<RootEvent> = root.org(name).tick(2).events().try_collect().await.unwrap_or_else(|e| panic!("org({name:?}): {e}"));
        assert_eq!(evs.len(), 2, "org({name:?})");
        match &evs[1] {
            RootEvent::Org { name: n, event } => {
                assert_eq!(&**n, name);
                assert!(matches!(**event, OrgEvent::Tick(N { n: 1 })), "{event:?}");
            }
            other => panic!("{other:?}"),
        }
        // and below it
        let s: Vec<RootEvent> = root.org(name).stats().tick(1).events().try_collect().await.unwrap();
        assert_eq!(s.len(), 1);
    }
}

/// A name the service's own path grammar refuses (`[a-z][a-z0-9_]*`, plexus-auth-core's
/// `MethodPath` segment) fails on the client, nothing sent.
#[tokio::test]
async fn a_name_outside_the_services_grammar_is_refused() {
    let t = Recording::default();
    let root = RootClient::new(&t);
    for name in [" ", "a b", "ａcme", "LEFT", "x\n", "-", "9lives", "acme-co"] {
        refused(root.org(name).tick(1).events().try_collect().await, name, NameRefusal::NotASegment);
    }
    assert!(t.0.lock().unwrap().is_empty(), "sent {:?}", t.0.lock().unwrap());
    // `..` and a dotted name are dotted
    refused(root.org("..").tick(1).events().try_collect().await, "..", NameRefusal::Dotted);
}

/// The client's rule and the service's agree, name by name. (A dotted name
/// the grammar takes, `a.b`, the client refuses for another reason: the
/// service would read two segments and route it past the instance.)
#[test]
fn the_clients_segment_rule_is_the_services() {
    use plexus_core::plexus::MethodPath;
    for name in ["", " ", "a b", "ａcme", "LEFT", "x\n", "-", "..", "acme", "a_1", "_a", "9lives", "acme-co", "o"] {
        let service = MethodPath::try_new(format!("root.{name}.tick")).is_ok();
        let client = plexus_client::Route::ROOT.instance("root", name, &[]).method(MethodRef::new("org", "tick")).is_ok();
        assert_eq!(client, service, "{name:?}: client {client}, service {service}");
    }
}

#[tokio::test]
async fn an_empty_name_is_refused() {
    let t = Recording::default();
    let root = RootClient::new(&t);
    refused(root.org("").tick(1).events().try_collect().await, "", NameRefusal::Empty);
    assert!(t.0.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_good_name_is_sent_where_it_says() {
    let t = Recording::default();
    let root = RootClient::new(&t);
    let evs: Vec<RootEvent> = root.org("acme").tick(1).events().try_collect().await.unwrap();
    assert_eq!(evs.len(), 1);
    assert_eq!(*t.0.lock().unwrap(), ["root.acme.tick"]);
}

#[test]
fn the_refusal_names_the_name_and_why() {
    let e = ClientError::InvalidName { name: "acme.stats".into(), reason: NameRefusal::Dotted };
    let s = e.to_string();
    assert!(s.contains("acme.stats") && s.contains('.'), "{s}");
}
