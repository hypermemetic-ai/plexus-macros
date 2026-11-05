//! RETURNS-46 over BIDIR-6: a many-item method with an input half, on a
//! static child under a dynamic one, is lifted into the root's aggregate and
//! named by its alias; its `MethodRef` carries the client's `RoutePath`, and
//! an instance name the client refuses sends nothing, input or call.

use std::sync::{Arc, Mutex};

use aggregate_probe::*;
use futures::{StreamExt, TryStreamExt};
use plexus_client::{ClientError, InputItem, InputItems, ItemStream, MethodRef, NameRefusal, StreamItem, Transport};
use serde_json::Value;

/// Answers a call with an input half with each input item as a data item,
/// recording the path; a call without one with nothing.
#[derive(Default, Clone)]
struct Echo(Arc<Mutex<Vec<String>>>);

impl Transport for Echo {
    fn call(&self, method: MethodRef, _params: Value) -> ItemStream<'_> {
        self.0.lock().unwrap().push(method.qualified());
        Box::pin(futures::stream::iter([Ok(StreamItem::done())]))
    }

    fn call_with_input<'a>(&'a self, method: MethodRef, _params: Value, input: InputItems<'a>) -> ItemStream<'a> {
        self.0.lock().unwrap().push(method.qualified());
        Box::pin(
            input
                .map(|i| match i {
                    Ok(InputItem::Json(v)) => Ok(StreamItem::data(v)),
                    Ok(other) => panic!("{other:?}"),
                    Err(e) => Err(e),
                })
                .chain(futures::stream::iter([Ok(StreamItem::done())])),
        )
    }
}

fn held<'a, T: Transport, L: plexus_client::Lift<StatsEvent>>(
    c: &'a StatsClient<T, L>,
    ns: Vec<N>,
) -> StatsDoubleStream<'a, L> {
    c.double(futures::stream::iter(ns))
}

#[tokio::test]
async fn an_input_half_is_lifted_and_named_like_any_method() {
    let t = Echo::default();
    let root = RootClient::new(&t);
    let org = root.org("acme");
    let stats = org.stats();
    let evs: Vec<RootEvent> = held(&stats, vec![N { n: 1 }, N { n: 2 }]).events().try_collect().await.unwrap();
    let lines: Vec<String> = evs.iter().map(|e| serde_json::to_string(e).unwrap()).collect();
    assert_eq!(
        lines,
        [
            r#"{"method":"org","event":{"name":"acme","event":{"method":"stats","event":{"method":"double","event":{"n":1}}}}}"#,
            r#"{"method":"org","event":{"name":"acme","event":{"method":"stats","event":{"method":"double","event":{"n":2}}}}}"#,
        ]
    );
    assert_eq!(*t.0.lock().unwrap(), ["root.acme.stats.double"]);
}

#[tokio::test]
async fn a_refused_instance_sends_no_input() {
    let t = Echo::default();
    let root = RootClient::new(&t);
    let org = root.org("LEFT");
    let stats = org.stats();
    let r: Result<Vec<RootEvent>, ClientError> = stats.double(futures::stream::iter(vec![N { n: 1 }])).events().try_collect().await;
    match r {
        Err(ClientError::InvalidName { name, reason: NameRefusal::NotASegment }) => assert_eq!(name, "LEFT"),
        other => panic!("{other:?}"),
    }
    assert!(t.0.lock().unwrap().is_empty());
}
