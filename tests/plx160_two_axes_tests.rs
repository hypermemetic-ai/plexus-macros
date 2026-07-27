//! PLX-160 — `ChildEdge` carries two axes, and the macro reads both.
//!
//! The defect: `ChildEdge` had three variants answering two independent
//! questions — what SHAPE is this child (one named child, or an indexed
//! family?) and how is it DELIVERED (embedded here, or fetched later?). Three
//! variants cannot cover four cells, and `ir_parse/signature.rs::child_edge`
//! resolved the shortfall by matching on `ChildMethodKind` (the *arity*, i.e.
//! delivery) **first** and consulting `info.list_fn` (the *shape*) only inside
//! the static arm. A dynamic child declared `#[child(list = "…")]` therefore
//! emitted a lazy single edge and discarded `list_method`, `id_field` and
//! `path_template` **with no diagnostic**.
//!
//! What makes this measurable rather than a matter of taste: the `#[child]`
//! grammar was ALREADY a clean product of the two axes.
//!
//! | | `fn ns(&self) -> C` | `fn ns(&self, name: &str) -> Option<C>` |
//! |---|---|---|
//! | bare `#[child]` | single, embedded | single, lazy |
//! | `#[child(list = "…")]` | indexed, embedded | indexed, lazy |
//!
//! The declaration surface always had four cells. Only the IR had three.

use async_stream::stream;
use futures::Stream;
use plexus_core::ir::{ActivationIr, ChildShape};
use plexus_core::plexus::ChildRouter;

// ===========================================================================
// One activation declaring all four cells
// ===========================================================================

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(namespace = "leaf", version = "1.0.0", crate_path = "plexus_core")]
impl Leaf {
    /// A method, so the leaf is not empty.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }
}

// The four cells get four activations only because of PRE-EXISTING per-impl
// constraints in the `#[child]` grammar — "at most one dynamic #[child] method"
// and "at most one list method per activation impl". Both are about routing
// arity and are orthogonal to PLX-160's two axes.

#[derive(Clone)]
struct SingleEmbedded;

#[plexus_macros::activation(
    namespace = "single_embedded",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl SingleEmbedded {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// (single, embedded) — the old `Static`.
    #[plexus_macros::child]
    fn kid(&self) -> Leaf {
        Leaf
    }
}

#[derive(Clone)]
struct SingleLazy;

#[plexus_macros::activation(
    namespace = "single_lazy",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl SingleLazy {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// (single, lazy) — the old `Dynamic`.
    #[plexus_macros::child]
    fn kid(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }
}

#[derive(Clone)]
struct IndexedEmbedded;

#[plexus_macros::activation(
    namespace = "indexed_embedded",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl IndexedEmbedded {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// (indexed, embedded) — the old `Indexed`.
    #[plexus_macros::child(list = "kid_ids")]
    fn kid(&self) -> Leaf {
        Leaf
    }

    fn kid_ids(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "k1".to_string(); }
    }
}

#[derive(Clone)]
struct IndexedLazy;

#[plexus_macros::activation(
    namespace = "indexed_lazy",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl IndexedLazy {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// (indexed, lazy) — **the cell that had no variant**, and the shape
    /// `solar.body`, `claudecode.session` and `cone.of` have always declared.
    #[plexus_macros::child(list = "kid_ids")]
    fn kid(&self, _id: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    fn kid_ids(&self) -> impl Stream<Item = String> + Send + '_ {
        stream! { yield "k1".to_string(); }
    }
}

fn edge<'a>(ir: &'a ActivationIr, ns: &str) -> &'a plexus_core::ir::ChildEdge {
    ir.child(ns).unwrap_or_else(|| panic!("no `{ns}` edge"))
}

/// **c1** — all four combinations are representable, and the macro produces
/// each one from a declaration. None is a special case.
#[test]
fn the_four_cells_are_all_reachable_and_none_is_special_cased() {
    let se = SingleEmbedded::activation_ir();
    let sl = SingleLazy::activation_ir();
    let ie = IndexedEmbedded::activation_ir();
    let il = IndexedLazy::activation_ir();

    let cells: Vec<(&str, bool, bool)> = vec![
        ("single/embedded", edge(&se, "kid").is_indexed(), edge(&se, "kid").is_lazy()),
        ("single/lazy", edge(&sl, "kid").is_indexed(), edge(&sl, "kid").is_lazy()),
        ("indexed/embedded", edge(&ie, "kid").is_indexed(), edge(&ie, "kid").is_lazy()),
        ("indexed/lazy", edge(&il, "kid").is_indexed(), edge(&il, "kid").is_lazy()),
    ];

    assert_eq!(
        cells,
        vec![
            ("single/embedded", false, false),
            ("single/lazy", false, true),
            ("indexed/embedded", true, false),
            ("indexed/lazy", true, true),
        ],
        "the two axes must vary independently — four declarations, four cells"
    );

    // Non-vacuity: the four cells are four DISTINCT §4.6 preimages, so this is
    // not four spellings of one thing.
    let mut hashes: Vec<String> = vec![
        edge(&se, "kid").edge_hash(),
        edge(&sl, "kid").edge_hash(),
        edge(&ie, "kid").edge_hash(),
        edge(&il, "kid").edge_hash(),
    ];
    hashes.sort();
    hashes.dedup();
    assert_eq!(hashes.len(), 4, "the four cells must not collide in §4.6");
}

/// **c2** — the enumeration facts survive on a *lazy* family. This is the exact
/// declaration whose `list_method`, `id_field` and `path_template` used to be
/// discarded, and the assertion is that they are now on the edge.
#[test]
fn a_lazy_child_declared_with_list_keeps_every_enumeration_fact() {
    let ir = IndexedLazy::activation_ir();
    let e = edge(&ir, "kid");

    assert!(e.is_lazy(), "delivery is unchanged by the shape axis");

    let ChildShape::Indexed {
        list_method,
        search_method,
        id_field,
        path_template,
    } = &e.shape
    else {
        panic!("`list =` on a dynamic child must produce an indexed SHAPE, got {:?}", e.shape)
    };

    assert_eq!(list_method, "indexed_lazy.kid_ids");
    assert_eq!(search_method, &None);
    assert_eq!(id_field, "name");
    assert_eq!(path_template, "kid/{id}");

    // §5.1(c) — "know an instance's shape". A lazy family carries it as a
    // reference: the advertised hash of the template, which §5.1's sufficiency
    // clause makes fetchable. It is deliberately NOT embedded — that is the
    // other axis, and PLX-148 already chose lazy delivery for these three.
    assert!(e.child().is_none(), "a lazy family embeds nothing (§5.2)");
    assert_eq!(
        e.advertised_hash(),
        Leaf::activation_ir().hash,
        "the template reference is the declared child type's own node hash"
    );
    assert_eq!(e.advertised_hash().len(), 64);
}

/// The other three cells did not move while the fourth was added.
#[test]
fn the_three_pre_existing_cells_are_unchanged() {
    let se = SingleEmbedded::activation_ir();
    let plain = edge(&se, "kid");
    assert!(!plain.is_indexed() && !plain.is_lazy());
    let embedded = plain.child().expect("still embedded");
    assert_eq!(embedded.namespace, "kid");
    assert!(embedded.method("ping").is_some());
    assert!(embedded.ir_hash.is_none(), "§3.3 — a node, not a document");

    let sl = SingleLazy::activation_ir();
    let registered = edge(&sl, "kid");
    assert!(!registered.is_indexed() && registered.is_lazy());
    assert_eq!(registered.advertised_hash(), Leaf::activation_ir().hash);
    assert!(registered.child().is_none());

    let ie = IndexedEmbedded::activation_ir();
    let family = edge(&ie, "kid");
    assert!(family.is_indexed() && !family.is_lazy());
    assert!(
        family.child().is_some(),
        "an embedded family still carries its template inline"
    );
}

/// **c3, the honest half.** `search` without `list` is NOT a droppable fact and
/// NOT a compile error: CHILD-4 made the two opt-ins independent and the
/// declaration is honoured in full on the routing surface. What it cannot do is
/// ride an IR edge — RFC 002 §5.1 makes the enumeration method MANDATORY and the
/// search method the only optional one of the five — so the backend has a
/// capability the DOCUMENT cannot state.
///
/// That is a §5.1 gap, not a macro bug. This test exists so it stays loud
/// instead of becoming another silent drop; see the report's residual R2.
#[tokio::test]
async fn search_only_is_honoured_by_the_router_and_unrepresentable_in_the_document() {
    let ir = SearchOnly::activation_ir();
    let e = edge(&ir, "planet");

    assert!(
        !e.is_indexed(),
        "§5.1 has no searchable-but-not-enumerable family, so the edge is Single"
    );
    // The routing surface, where the declaration IS honoured in full: the
    // sibling `find` is reachable through `search_children`, so nothing was
    // dropped — it simply has nowhere to go in the document.
    assert!(
        SearchOnly.search_children("x").await.is_some(),
        "the search opt-in is honoured by the router"
    );
    assert!(SearchOnly.list_children().await.is_none());
}

#[derive(Clone)]
struct SearchOnly;

#[plexus_macros::activation(
    namespace = "search_only",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl SearchOnly {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Dynamic child lookup with search opt-in only.
    #[plexus_macros::child(search = "find")]
    fn planet(&self, _name: &str) -> Option<Leaf> {
        Some(Leaf)
    }

    fn find(&self, query: &str) -> impl Stream<Item = String> + Send + '_ {
        let q = query.to_string();
        stream! { yield q; }
    }
}
