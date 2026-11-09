//! CHILD-4 runtime acceptance tests for the `list = "..."` and
//! `search = "..."` arguments on `#[plexus_macros::child]`.
//!
//! This file exercises every row of the capability matrix defined in the
//! CHILD-4 ticket, verifies the boxing behavior for both
//! `impl Stream<Item = String>` and `BoxStream<'_, String>` return shapes,
//! and confirms the pinned decision that static children are always listed.
//!
//! Compile-fail fixtures for `list = nonexistent` / signature mismatch /
//! etc. live in `tests/compile/` and are driven from
//! `tests/child_compile_tests.rs`.

use async_stream::stream;
use futures::stream::{BoxStream, Stream, StreamExt};
use plexus_core::plexus::{ChildCapabilities, ChildRouter};

// ---------------------------------------------------------------------------
// Common leaf used as the child type everywhere below.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(
    namespace = "leaf",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Leaf {
    /// Health check.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

// ---------------------------------------------------------------------------
// Row: `#[child]` only (no args), dynamic method → empty caps, None everywhere.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BareDynamic;

#[plexus_macros::activation(
    namespace = "bare_dynamic",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl BareDynamic {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// A bare dynamic child, no `list`/`search` args — the router must
    /// refuse to list or search.
    #[plexus_macros::child]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }
}

#[tokio::test]
async fn bare_dynamic_has_empty_capabilities() {
    let hub = BareDynamic;
    assert_eq!(hub.capabilities(), ChildCapabilities::empty());
    assert!(hub.list_children().await.is_none());
    assert!(hub.search_children("anything").await.is_none());
}

// ---------------------------------------------------------------------------
// Row: `#[child]` only, static methods only → LIST; list yields names.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct StaticOnly {
    a: Leaf,
    b: Leaf,
}

#[plexus_macros::activation(
    namespace = "static_only",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl StaticOnly {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Static child `alpha`.
    #[plexus_macros::child]
    fn alpha(&self) -> Leaf {
        self.a.clone()
    }

    /// Static child `beta`.
    #[plexus_macros::child]
    fn beta(&self) -> Leaf {
        self.b.clone()
    }
}

#[tokio::test]
async fn static_only_has_list_capability_and_streams_names() {
    let hub = StaticOnly { a: Leaf, b: Leaf };
    assert_eq!(hub.capabilities(), ChildCapabilities::LIST);
    let names: Vec<String> = hub
        .list_children()
        .await
        .expect("static-only activation must expose list_children")
        .collect()
        .await;
    assert_eq!(names, vec!["alpha".to_string(), "beta".to_string()]);

    // No search sibling ⇒ search_children returns None.
    assert!(hub.search_children("zzz").await.is_none());
}

// ---------------------------------------------------------------------------
// Row: `#[child(list = names)]` on dynamic method → LIST, stream from the
// named method (plus static names when present). This test covers the
// dynamic-only sub-case.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DynamicList;

#[plexus_macros::activation(
    namespace = "dynamic_list",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl DynamicList {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Dynamic child lookup — opts in to listing via the sibling `names`.
    #[plexus_macros::child(list = names)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    /// Stream the dynamically-known names.
    fn names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! {
            yield "earth".to_string();
            yield "mars".to_string();
        }
    }
}

#[tokio::test]
async fn dynamic_list_streams_from_named_method() {
    let hub = DynamicList;
    assert_eq!(hub.capabilities(), ChildCapabilities::LIST);
    let names: Vec<String> = hub
        .list_children()
        .await
        .expect("list opt-in must produce Some(stream)")
        .collect()
        .await;
    assert_eq!(names, vec!["earth".to_string(), "mars".to_string()]);

    // Search not opted in.
    assert!(hub.search_children("earth").await.is_none());
}

// ---------------------------------------------------------------------------
// Row: `#[child(list = "...")]` on dynamic method + static children → LIST,
// static names streamed first, then dynamic names appended.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MixedList {
    alpha_leaf: Leaf,
}

#[plexus_macros::activation(
    namespace = "mixed_list",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl MixedList {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Static child `alpha` — always listed.
    #[plexus_macros::child]
    fn alpha(&self) -> Leaf {
        self.alpha_leaf.clone()
    }

    /// Dynamic child with explicit list opt-in.
    #[plexus_macros::child(list = dynamic_names)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    /// Dynamic names streamed after the static names.
    fn dynamic_names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! {
            yield "gamma".to_string();
        }
    }
}

#[tokio::test]
async fn mixed_list_appends_dynamic_after_static() {
    let hub = MixedList { alpha_leaf: Leaf };
    assert_eq!(hub.capabilities(), ChildCapabilities::LIST);
    let names: Vec<String> = hub
        .list_children()
        .await
        .expect("mixed static + dynamic list must yield Some(stream)")
        .collect()
        .await;
    assert_eq!(
        names,
        vec!["alpha".to_string(), "gamma".to_string()],
        "static names first, dynamic names appended"
    );
}

// ---------------------------------------------------------------------------
// Row: `#[child(search = find)]` on dynamic method → SEARCH only; no
// static children so list_children remains None.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct DynamicSearch;

#[plexus_macros::activation(
    namespace = "dynamic_search",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl DynamicSearch {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Dynamic child lookup with search opt-in only.
    #[plexus_macros::child(search = find)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    /// Deterministic search: echoes the query back plus a companion.
    fn find(&self, query: &str) -> impl Stream<Item = String> + Send + '_ {
        let q = query.to_string();
        stream! {
            yield q.clone();
            yield format!("{}-suffix", q);
        }
    }
}

#[tokio::test]
async fn dynamic_search_has_search_capability_only() {
    let hub = DynamicSearch;
    assert_eq!(hub.capabilities(), ChildCapabilities::SEARCH);
    // No list opt-in and no static children.
    assert!(hub.list_children().await.is_none());
    // Search delegates to the named sibling.
    let hits: Vec<String> = hub
        .search_children("foo")
        .await
        .expect("search opt-in must produce Some(stream)")
        .collect()
        .await;
    assert_eq!(hits, vec!["foo".to_string(), "foo-suffix".to_string()]);
}

// ---------------------------------------------------------------------------
// Row: `#[child(list = "...", search = "...")]` → LIST | SEARCH, both
// streams delegated.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct BothListAndSearch;

#[plexus_macros::activation(
    namespace = "both_list_search",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl BothListAndSearch {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    #[plexus_macros::child(list = all_names, search = find)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    fn all_names(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "x".to_string(); yield "y".to_string(); }
    }

    fn find(&self, query: &str) -> impl Stream<Item = String> + Send + '_ {
        let q = query.to_string();
        stream! { yield q; }
    }
}

#[tokio::test]
async fn both_list_and_search_set_both_flags() {
    let hub = BothListAndSearch;
    assert_eq!(
        hub.capabilities(),
        ChildCapabilities::LIST | ChildCapabilities::SEARCH
    );

    let names: Vec<String> = hub.list_children().await.unwrap().collect().await;
    assert_eq!(names, vec!["x".to_string(), "y".to_string()]);

    let hits: Vec<String> = hub.search_children("q").await.unwrap().collect().await;
    assert_eq!(hits, vec!["q".to_string()]);
}

// ---------------------------------------------------------------------------
// Return-shape parity: a list method returning `impl Stream<Item = String>`
// and a search method returning `BoxStream<'_, String>` on the same
// activation. Both must compile and produce identical behavior to the pure
// `impl Stream` version.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct MixedReturnShapes;

#[plexus_macros::activation(
    namespace = "mixed_return_shapes",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl MixedReturnShapes {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    #[plexus_macros::child(list = names_impl, search = find_box)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    /// `impl Stream<Item = String>` return shape — should be boxed by the macro.
    fn names_impl(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! {
            yield "impl-one".to_string();
            yield "impl-two".to_string();
        }
    }

    /// `BoxStream<'_, String>` return shape — already boxed; macro must pass
    /// it through (its generated `.boxed()` rewrap is a no-op).
    fn find_box(&self, query: &str) -> BoxStream<'_, String> {
        let q = query.to_string();
        let inner = stream! {
            yield format!("box-{}", q);
            yield format!("box-{}-extra", q);
        };
        inner.boxed()
    }
}

#[tokio::test]
async fn mixed_return_shapes_compile_and_behave_identically() {
    let hub = MixedReturnShapes;
    assert_eq!(
        hub.capabilities(),
        ChildCapabilities::LIST | ChildCapabilities::SEARCH
    );

    let names: Vec<String> = hub.list_children().await.unwrap().collect().await;
    assert_eq!(
        names,
        vec!["impl-one".to_string(), "impl-two".to_string()],
    );

    let hits: Vec<String> = hub.search_children("q").await.unwrap().collect().await;
    assert_eq!(hits, vec!["box-q".to_string(), "box-q-extra".to_string()]);
}

// ---------------------------------------------------------------------------
// Async sibling methods: the macro must `.await` async siblings before
// wrapping. Covers acceptance criterion 9's "identical behavior" property
// across sync and async sibling methods.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct AsyncSiblings;

#[plexus_macros::activation(
    namespace = "async_siblings",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl AsyncSiblings {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    #[plexus_macros::child(list = names, search = find)]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    async fn names(&self) -> BoxStream<'_, String> {
        stream! {
            yield "async-a".to_string();
            yield "async-b".to_string();
        }
        .boxed()
    }

    async fn find(&self, query: &str) -> impl Stream<Item = String> + Send + '_ {
        let q = query.to_string();
        stream! { yield format!("async-hit:{}", q); }
    }
}

#[tokio::test]
async fn async_siblings_are_awaited_by_macro() {
    let hub = AsyncSiblings;
    assert_eq!(
        hub.capabilities(),
        ChildCapabilities::LIST | ChildCapabilities::SEARCH
    );

    let names: Vec<String> = hub.list_children().await.unwrap().collect().await;
    assert_eq!(
        names,
        vec!["async-a".to_string(), "async-b".to_string()],
    );

    let hits: Vec<String> = hub.search_children("z").await.unwrap().collect().await;
    assert_eq!(hits, vec!["async-hit:z".to_string()]);
}
