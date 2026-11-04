//! RETURNS-46 review: an instance name that is empty, holds a `.`, or is a
//! static sibling's segment or a method of the parent would be routed
//! somewhere else than the instance it names. The accessor stays
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

#[tokio::test]
async fn a_method_of_the_parent_is_refused() {
    let t = Recording::default();
    let root = RootClient::new(&t);
    refused(root.org("tick").tick(1).events().try_collect().await, "tick", NameRefusal::Method);
    refused(root.org("schema").tick(1).events().try_collect().await, "schema", NameRefusal::Method);
    let left = root.left();
    match left.dir("tick").tick(1).events().try_collect::<Vec<RootEvent>>().await {
        Err(ClientError::InvalidName { name, reason: NameRefusal::Method }) => assert_eq!(name, "tick"),
        other => panic!("{other:?}"),
    }
    assert!(t.0.lock().unwrap().is_empty());
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
