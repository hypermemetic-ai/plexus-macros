//! RETURNS-36 (review #8): the activation's trait declares each method's own
//! return type — all six forms. RETURNS-47: a trait only when named
//! (`trait = "Conformance"` here), implemented by the activation; the client
//! implements no trait (its six forms, as `Result`s, are `tests/client.rs`'s).

use conformance::*;
use futures::StreamExt;
use plexus_client::{Note, Progress};

fn through(c: &dyn Conformance) -> &dyn Conformance {
    c
}

async fn all_six(c: &dyn Conformance) {
    assert_eq!(c.ping().await, Pong { message: "pong".into() });
    assert_eq!(c.lookup("a1".into()).await, Ok(Thing { id: "a1".into(), name: "thing a1".into() }));
    assert_eq!(c.lookup("missing".into()).await, Err(LookupError::NotFound { id: "missing".into() }));
    let ticks: Vec<Tick> = c.count(3).collect().await;
    assert_eq!(ticks, vec![Tick { n: 1 }, Tick { n: 2 }, Tick { n: 3 }]);
    let rows: Vec<Result<Row, ScanError>> = c.scan("a".into()).collect().await;
    assert_eq!(rows.len(), 3);
    assert!(rows[1].is_err(), "an err item, and the scan carries on");
    let load: Vec<Result<Progress<Loaded, LoadStep>, LoadError>> = c.load(2).collect().await;
    assert_eq!(load.last(), Some(&Ok(Progress::Item(Loaded { items: 2 }))));
    assert_eq!(load.first(), Some(&Ok(Progress::Update(LoadStep::Batch { done: 1, of: 2 }))));
    let warm: Vec<Progress<Warmed>> = c.warm().collect().await;
    assert_eq!(warm.first(), Some(&Progress::Update(Note::new("cache cold"))));
    assert_eq!(warm.last(), Some(&Progress::Item(Warmed { entries: 2 })));
}

#[tokio::test]
async fn the_activation_is_its_trait() {
    all_six(through(&ConformanceActivation)).await;
}
