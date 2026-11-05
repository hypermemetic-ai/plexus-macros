//! RETURNS-46: each many-item method has a named stream type,
//! `{Ns}{Method}Stream<'a, L = Root>` — `MethodStream<'a, I, P, {Ns}Event,
//! L, …>` with its zero-sized variant hidden — so a struct field or a return
//! type can name a call's stream: a root method, and a method on a static
//! and on a dynamic child's client.

use aggregate_probe::*;
use futures::TryStreamExt;
use plexus_client::{Lift, Transport};

/// Calls held in a struct: the root's own, and a static and a dynamic
/// child's, each lifted by its accessor (`L` left to the caller).
struct Held<'a, Ls, Lo> {
    root: RootTickStream<'a>,
    left: SideTickStream<'a, Ls>,
    org: OrgTickStream<'a, Lo>,
}

fn root_ticks<T: Transport>(c: &RootClient<T>) -> RootTickStream<'_> {
    c.tick(2)
}

fn side_ticks<T: Transport, L: Lift<SideEvent>>(c: &SideClient<T, L>) -> SideTickStream<'_, L> {
    c.tick(2)
}

fn org_ticks<T: Transport, L: Lift<OrgEvent>>(c: &OrgClient<T, L>) -> OrgTickStream<'_, L> {
    c.tick(2)
}

#[tokio::test]
async fn a_stream_is_named_by_its_alias() {
    let f = Fast(2);
    let root = RootClient::new(&f);
    let left = root.left();
    let org = root.org("acme");
    let held = Held { root: root_ticks(&root), left: side_ticks(&left), org: org_ticks(&org) };
    let r: Vec<RootEvent> = held.root.events().try_collect().await.unwrap();
    let l: Vec<RootEvent> = held.left.events().try_collect().await.unwrap();
    let o: Vec<RootEvent> = held.org.events().try_collect().await.unwrap();
    assert_eq!((r.len(), l.len(), o.len()), (2, 2, 2));
    assert!(matches!(l[0], RootEvent::Left(SideEvent::Tick(N { n: 0 }))), "{:?}", l[0]);
    assert!(matches!(&o[1], RootEvent::Org { name, .. } if name.as_ref() == "acme"), "{:?}", o[1]);
    // the child's own client, its default lift
    let side = SideClient::at(&f, "root.left");
    let own: SideTickStream<'_> = side.tick(1);
    let own: Vec<SideEvent> = own.events().try_collect().await.unwrap();
    assert_eq!(own.len(), 2, "Fast(2) answers any call with two items");
}
