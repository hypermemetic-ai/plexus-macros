//! PLX-150 — a synthesized child edge advertises the child's real hash.
//!
//! `codegen/activation.rs` emitted `hash: ::std::string::String::new()` in the
//! `plugin_children` block it synthesizes whenever an activation does not
//! hand-write one. That is the identical defect PLX-90 found and fixed on the
//! IR path (`ir_parse` emitted `hash: String::new()` for every Dynamic edge,
//! which connectome-hs's conformance checker rejected as an RFC 002 §5.1 MUST
//! violation); the legacy path survived because nobody re-checked it there.
//!
//! PLX-143 measured the consequence on a live wire: 22 of 23 advertised hashes
//! agreed with the child's self-report, and the one that did not was `orcha`
//! advertising `pm` with `""` while `orcha.pm.schema` self-reported
//! `50bfa7415dec30dd`. Rust serializes the empty string and Haskell decodes
//! strictly, so `""` arrived as a VALUE, not a decode error — synapse looked up
//! on the advertised hash and inserted on the fetched one, and `orcha.pm` was
//! permanently uncacheable, silently, forever.
//!
//! These run against the real macro over a real activation, and compare the
//! advertised value to the child's own `plugin_schema().hash` rather than to a
//! constant, so the test cannot pass by agreeing with itself.

use async_stream::stream;
use futures::stream::Stream;
use plexus_core::plexus::Activation;

#[derive(Clone)]
struct Mercury;

#[plexus_macros::activation(
    namespace = "mercury",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Mercury {
    /// Mercury's own method — gives it a non-trivial self hash.
    #[plexus_macros::method]
    async fn orbit(&self, days: u32) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield format!("{days}"); }
    }
}

#[derive(Clone)]
struct Venus;

#[plexus_macros::activation(
    namespace = "venus",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Venus {
    /// A DIFFERENT signature, so venus must not hash the same as mercury.
    #[plexus_macros::method]
    async fn rotate(&self, retrograde: bool) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield format!("{retrograde}"); }
    }
}

/// Two static `#[child]` accessors and NO hand-written `plugin_children`, so
/// the macro synthesizes one. This is the shape `orcha` has.
#[derive(Clone)]
struct Inner {
    mercury: Mercury,
    venus: Venus,
}

#[plexus_macros::activation(
    namespace = "inner_hub",
    version = "1.0.0",
    crate_path = "plexus_core"
)]
impl Inner {
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        stream! { yield "pong".into(); }
    }

    /// Static child — mercury.
    #[plexus_macros::child]
    fn mercury(&self) -> Mercury {
        self.mercury.clone()
    }

    /// Static child — venus.
    #[plexus_macros::child]
    fn venus(&self) -> Venus {
        self.venus.clone()
    }
}

fn hub() -> Inner {
    Inner { mercury: Mercury, venus: Venus }
}

#[allow(deprecated)] // `PluginSchema::children` is the wire field under test.
fn children(schema: &plexus_core::plexus::PluginSchema) -> &[plexus_core::plexus::ChildSummary] {
    schema.children.as_deref().expect("a hub populates children")
}

/// PLX-150 c1 — no synthesized child edge advertises an empty hash.
#[test]
fn every_synthesized_child_edge_advertises_a_non_empty_hash() {
    let schema = hub().plugin_schema();
    let kids = children(&schema);
    assert_eq!(kids.len(), 2, "both static children are advertised");
    for c in kids {
        assert!(
            !c.hash.is_empty(),
            "RFC 002 §5.1: child edge `{}` MUST advertise a hash. `\"\"` is not \
             one — it decodes cleanly and can never match, which is exactly why \
             this survived unnoticed on the legacy path",
            c.namespace,
        );
    }
}

/// The advertised value is the child's OWN hash — the value a consumer gets
/// back when it fetches `<parent>.<child>.schema`. This is the property
/// synapse's cache depends on: it looks up and inserts on one key.
#[test]
fn the_advertised_hash_equals_the_childs_self_reported_hash() {
    let h = hub();
    let schema = h.plugin_schema();
    let kids = children(&schema);

    let mercury = kids.iter().find(|c| c.namespace == "mercury").unwrap();
    let venus = kids.iter().find(|c| c.namespace == "venus").unwrap();

    assert_eq!(mercury.hash, Mercury.plugin_schema().hash);
    assert_eq!(venus.hash, Venus.plugin_schema().hash);

    // And they are genuinely distinct, so the assertions above are not both
    // trivially satisfied by one shared constant.
    assert_ne!(
        mercury.hash, venus.hash,
        "two children with different method surfaces must advertise different \
         hashes — an emitter that collapsed them onto one value would break \
         caching in the opposite direction"
    );
}

/// PLX-150 c4 — what must NOT move.
///
/// The child edge's hash feeds `children_hash` and therefore the composite
/// `hash`; that movement is the point of the fix. What must not move is the
/// METHOD surface: `compute_method_hash` reads name, description, params and
/// return types, and `self_hash` folds nothing but method hashes.
#[test]
fn no_method_hash_and_no_self_hash_depends_on_the_child_edge() {
    let schema = hub().plugin_schema();

    // `self_hash` is a fold of method hashes only, so it is computable from
    // the method list alone and cannot see the child edge.
    let recomputed = {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut h = DefaultHasher::new();
        for m in &schema.methods {
            m.hash.hash(&mut h);
        }
        format!("{:016x}", h.finish())
    };
    assert_eq!(
        schema.self_hash, recomputed,
        "self_hash must remain a pure fold of method hashes"
    );

    // The composite DOES depend on the children — that is the wire-visible
    // change, stated rather than hidden.
    assert!(
        schema.children_hash.is_some(),
        "a hub folds a children_hash, and it is now built from real values"
    );
}

/// A leaf that never had a child edge is untouched: no `children`, no
/// `children_hash`, so its composite hash is unchanged by PLX-150. This is the
/// blast-radius floor — the overwhelming majority of activations.
#[test]
fn a_leaf_activation_is_unaffected() {
    let schema = Mercury.plugin_schema();
    assert!(schema.children_hash.is_none(), "a leaf folds no children");
    assert_eq!(
        schema.hash,
        {
            use std::collections::hash_map::DefaultHasher;
            use std::hash::{Hash, Hasher};
            let mut h = DefaultHasher::new();
            schema.self_hash.hash(&mut h);
            format!("{:016x}", h.finish())
        },
        "a leaf's composite is its self_hash alone"
    );
}
