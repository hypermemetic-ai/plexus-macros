//! PLX-113 — `#[activation]` can declare its RFC 002 §3.3 root facts.
//!
//! Three independent conformance runs (PLX-90, PLX-93, PLX-106) returned
//! CONFORMANT with the same two SHOULD advisories on every macro-produced
//! document: no `backend_name` (§3.3) and no reply-channel method (§7.6).
//! PLX-93 found the cause — `activation_ir!` accepts both arguments and
//! `#[activation]` accepted neither, so `activation_ir_fn` passed a hard-coded
//! `None, None`. This file is the acceptance suite for closing that.
//!
//! What is asserted here, and why each assertion is the one that matters:
//!
//! 1. **The macro can say it** (c1). An `#[activation]` declaring
//!    `backend_name` and `respond_method` produces an `ActivationIr` carrying
//!    both; one declaring neither is byte-identical to what it produced before
//!    this feature existed, pinned by literal digests rather than by a
//!    same-run comparison.
//! 2. **The facts stay at the root** (c2, RFC §3.3, pinned by PLX-90's c3). An
//!    embedded `#[child]` of a root that declares both carries neither, and the
//!    same child standing alone hashes identically.
//! 3. **No hash moves** (the load-bearing one). §4.6 puts the root facts in the
//!    *document* preimage and deliberately not in the *activation* preimage.
//!    So two roots identical except that one declares both root facts must
//!    agree on every method hash and on every activation hash in the tree, and
//!    differ **only** in `ir_hash`. That is asserted here in both directions:
//!    equality where §4.6 promises it, and inequality where §4.5 requires it
//!    (changing the backend identity must invalidate a consumer's cached
//!    document).
//!
//! Run with: `cargo test --test plx113_root_facts_tests`.

#![allow(dead_code)]

use futures::stream::Stream;
use plexus_core::ir::{ActivationIr, ChildEdge};

// ===========================================================================
// Fixtures
//
// Two roots that are IDENTICAL in every field that §4.6's activation preimage
// reads — same namespace, same version, same description, same method, same
// Static child of the same leaf type — and differ ONLY in whether they declare
// the §3.3 root facts. That is what makes the hash comparison below a test of
// §4.6's preimage rather than a test of two unrelated documents.
// ===========================================================================

#[derive(Clone)]
struct Leaf;

#[plexus_macros::activation(
    namespace = "plx113_leaf",
    version = "1.0.0",
    description = "A leaf activation.",
    crate_path = "plexus_core"
)]
impl Leaf {
    /// Say pong.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "pong".to_string(); }
    }
}

/// The root that declares its §3.3 root facts — what PLX-113 adds.
#[derive(Clone)]
struct Declaring {
    plx113_leaf: Leaf,
}

#[plexus_macros::activation(
    namespace = "plx113_root",
    version = "1.0.0",
    description = "A root that declares its identity and its reply channel.",
    backend_name = "plx113-backend",
    respond_method = "respond",
    crate_path = "plexus_core"
)]
impl Declaring {
    /// List the sessions.
    #[plexus_macros::method]
    async fn list(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "s1".to_string(); }
    }

    /// A Static edge — an embedded subtree.
    #[plexus_macros::child]
    fn plx113_leaf(&self) -> Leaf {
        self.plx113_leaf.clone()
    }
}

/// The same root, declaring neither — the shape every `#[activation]` had
/// before PLX-113 and the shape every one that does not opt in keeps.
#[derive(Clone)]
struct Bare {
    plx113_leaf: Leaf,
}

#[plexus_macros::activation(
    namespace = "plx113_root",
    version = "1.0.0",
    description = "A root that declares its identity and its reply channel.",
    crate_path = "plexus_core"
)]
impl Bare {
    /// List the sessions.
    #[plexus_macros::method]
    async fn list(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "s1".to_string(); }
    }

    /// A Static edge — an embedded subtree.
    #[plexus_macros::child]
    fn plx113_leaf(&self) -> Leaf {
        self.plx113_leaf.clone()
    }
}

fn static_child(ir: &ActivationIr) -> &ActivationIr {
    ir.children
        .iter()
        .find_map(|e| if e.is_indexed() { None } else { e.child() })
        .expect("the fixture has a single embedded child edge")
}

// ===========================================================================
// c1 — the macro can declare its root facts
// ===========================================================================

/// c1, first clause: an activation declaring both attributes produces an
/// `ActivationIr` carrying `backend_name` and the reply-channel method.
#[test]
fn an_activation_declaring_both_carries_both_root_facts() {
    let ir = Declaring::activation_ir();

    assert_eq!(
        ir.backend_name.as_deref(),
        Some("plx113-backend"),
        "§3.3 backend identity must reach the document"
    );
    assert_eq!(
        ir.respond_method.as_deref(),
        Some("respond"),
        "§3.3/§7.6 reply channel must reach the document"
    );

    // The three MANDATORY root facts are still there and still established by
    // `recompute_hashes`, not by the new attributes.
    assert!(ir.ir_version.is_some(), "§3.3: ir_version is a MUST");
    assert_eq!(
        ir.hash_algorithm.as_deref(),
        Some("CONNECTOME-HASH/1"),
        "§4.7: the hash construction is declared"
    );
    assert!(ir.ir_hash.is_some(), "§3.3/§4.5: the document hash is a MUST");
}

/// c1, second clause: one declaring neither is unchanged from today.
///
/// "Unchanged" is pinned by literal digests captured on `main` at `dea80dc`
/// (the merge base of `plx-113-root-facts`) — a same-run comparison against a
/// sibling fixture would only prove the two agree with each other, not that
/// either agrees with the code that shipped.
#[test]
fn an_activation_declaring_neither_is_byte_identical_to_before_plx113() {
    let ir = Bare::activation_ir();

    assert_eq!(ir.backend_name, None, "no declaration, no backend identity");
    assert_eq!(ir.respond_method, None, "no declaration, no reply channel");

    assert_eq!(
        ir.hash, BARE_ROOT_NODE_HASH,
        "the activation hash of an undeclaring root must not move"
    );
    assert_eq!(
        ir.ir_hash.as_deref(),
        Some(BARE_ROOT_IR_HASH),
        "the DOCUMENT hash of an undeclaring root must not move either"
    );
    assert_eq!(
        ir.methods[0].hash, LIST_METHOD_HASH,
        "no method hash may move"
    );
    assert_eq!(
        static_child(ir).hash,
        LEAF_NODE_HASH,
        "no embedded child hash may move"
    );

    // The control document for c3: the SAME fixture without the declarations,
    // so the checker's before/after is one variable rather than two documents.
    if let Ok(out) = std::env::var("PLX113_BARE_CONNECTOME_OUT") {
        let json = serde_json::to_string_pretty(ir).expect("the document serializes");
        std::fs::write(&out, &json).unwrap_or_else(|e| panic!("could not write {out}: {e}"));
    }
}

/// Digests measured on `main` @ `dea80dc` with this exact fixture, before any
/// PLX-113 change existed. Regenerate with `PLX113_PIN=1 cargo test --test
/// plx113_root_facts_tests -- --nocapture` and read the printed values.
const BARE_ROOT_NODE_HASH: &str =
    "d3a1dda4f5a588daff5a0910e8c0fe3158d59572b57a5c36984616f719133d66";
const BARE_ROOT_IR_HASH: &str =
    "ba6297397dbf828653ed3e01866ad452b55f92844e9b5c44fa234bfc06f2e175";
const LIST_METHOD_HASH: &str =
    "572fda40d3d9844b214a0ce490c642a97b0473c7c734d07cd7e49a6c8298b9ab";
const LEAF_NODE_HASH: &str =
    "a0cf84861ed5cf064429225f5b1c57a71d310718c392bc5581423def4656b867";

/// Prints the digests the pins above must hold. Not an assertion — the escape
/// hatch that keeps the pins honest when the fixture legitimately changes.
#[test]
fn print_the_pins() {
    if std::env::var("PLX113_PIN").is_err() {
        return;
    }
    let ir = Bare::activation_ir();
    println!("BARE_ROOT_NODE_HASH = {:?}", ir.hash);
    println!("BARE_ROOT_IR_HASH   = {:?}", ir.ir_hash);
    println!("LIST_METHOD_HASH    = {:?}", ir.methods[0].hash);
    println!("LEAF_NODE_HASH      = {:?}", static_child(ir).hash);
}

/// §3.5 — a present-but-empty optional is a MUST violation, not a declaration.
/// The macro refuses the empty spelling at the source; this pins the message.
/// The compile-fail fixture lives in `tests/compile/`.
#[test]
fn the_empty_spelling_is_rejected_at_the_source() {
    // Asserted by the trybuild case
    // `tests/compile/plx113_empty_root_fact.rs`; this test exists so the
    // reason is readable next to the rest of the suite.
    let ir = Declaring::activation_ir();
    assert!(
        !ir.backend_name.as_deref().unwrap_or_default().is_empty(),
        "§3.5: an emitted backend_name is never the empty string"
    );
    assert!(
        !ir.respond_method.as_deref().unwrap_or_default().is_empty(),
        "§3.5: an emitted respond_method is never the empty string"
    );
}

// ===========================================================================
// c2 — root facts stay at the root
// ===========================================================================

/// c2: an embedded child of a root that declares both carries NEITHER (§3.3),
/// and the same child standing alone hashes identically.
///
/// This is the invariant PLX-90's c3 pinned, re-asserted through the
/// `#[activation]` path now that that path can declare root facts at all. It is
/// what keeps a Static child's advertised hash comparable with the hash it
/// reports when fetched on its own (§4.6).
#[test]
fn an_embedded_child_of_a_declaring_root_carries_no_root_facts() {
    let root = Declaring::activation_ir();
    let child = static_child(root);

    assert_eq!(child.backend_name, None, "§3.3: not on a non-root");
    assert_eq!(child.respond_method, None, "§3.3: not on a non-root");
    assert_eq!(child.ir_version, None, "§3.3: not on a non-root");
    assert_eq!(child.hash_algorithm, None, "§3.3: not on a non-root");
    assert_eq!(child.ir_hash, None, "a node is not a document");

    // The standalone child DOES carry them — and its activation hash is the
    // same one it contributes when embedded.
    let standalone = Leaf::activation_ir();
    assert!(standalone.ir_version.is_some());
    assert!(standalone.ir_hash.is_some());
    assert_eq!(
        standalone.hash, child.hash,
        "§4.6: an activation hashes the same read as a root or as a subtree"
    );
}

// ===========================================================================
// The load-bearing one — declaring root facts moves no method or node hash
// ===========================================================================

/// §4.6 keeps the §3.3 root facts out of the ACTIVATION preimage and puts them
/// in the DOCUMENT preimage. Both halves are asserted, because either one
/// alone would be satisfied by a broken implementation:
///
/// * equality of every activation and method hash — the guarantee that adding
///   this feature cannot change a single existing document's digests, and that
///   an existing consumer's cached subtree hashes stay valid;
/// * inequality of `ir_hash` — §4.5 requires changing the backend identity or
///   the reply channel to invalidate a consumer's verification, so a
///   *document* hash that did NOT move would be the real bug.
#[test]
fn declaring_root_facts_moves_no_activation_or_method_hash() {
    let declaring = Declaring::activation_ir();
    let bare = Bare::activation_ir();

    // Same fixture shape, so the comparison is meaningful.
    assert_eq!(declaring.namespace, bare.namespace);
    assert_eq!(declaring.version, bare.version);
    assert_eq!(declaring.description, bare.description);
    assert_eq!(declaring.methods.len(), bare.methods.len());
    assert_eq!(declaring.children.len(), bare.children.len());

    // §4.6 — the activation preimage does not read the root facts.
    assert_eq!(
        declaring.hash, bare.hash,
        "the root's ACTIVATION hash must not move when root facts are declared"
    );
    for (d, b) in declaring.methods.iter().zip(bare.methods.iter()) {
        assert_eq!(d.name, b.name);
        assert_eq!(
            d.hash, b.hash,
            "method `{}`: no method hash may move (§4.6)",
            d.name
        );
    }
    assert_eq!(
        static_child(declaring).hash,
        static_child(bare).hash,
        "the embedded child's hash must not move"
    );

    // §4.5 — the DOCUMENT hash must move, or the root facts would be
    // unverifiable and a stale cache undetectable.
    assert_ne!(
        declaring.ir_hash, bare.ir_hash,
        "§4.5: the document hash must reflect the root facts"
    );
}

// ===========================================================================
// c3 — the export hook for the independent implementation
// ===========================================================================

/// The document connectome-hs checks: produced end to end by `#[activation]`,
/// declaring both root facts, with a Static child under it. §3.4 round trip is
/// asserted here so the exported bytes are known-lossless before the checker
/// ever sees them.
#[test]
fn export_a_declaring_macro_document_for_the_independent_checker() {
    let ir = Declaring::activation_ir();

    let json = serde_json::to_string_pretty(ir).expect("the document serializes");
    let back: ActivationIr = serde_json::from_str(&json).expect("the document decodes");
    assert_eq!(&back, ir, "§3.4: the round trip must be lossless");
    assert_eq!(back.ir_hash, ir.ir_hash);
    assert_eq!(back.hash, ir.hash);

    // §3.5 — the undeclared optionals must be ABSENT from the wire, not null
    // and not "". The two root facts that ARE declared must be present.
    let wire: serde_json::Value = serde_json::from_str(&json).expect("the document is JSON");
    assert_eq!(wire["backend_name"], serde_json::json!("plx113-backend"));
    assert_eq!(wire["respond_method"], serde_json::json!("respond"));
    let child = &wire["children"][0];
    assert!(
        child.get("backend_name").is_none() && child.get("respond_method").is_none(),
        "§3.3/§3.5: an embedded child carries no root facts, omitted not nulled: {child}"
    );

    if let Ok(out) = std::env::var("PLX113_CONNECTOME_OUT") {
        std::fs::write(&out, &json).unwrap_or_else(|e| panic!("could not write {out}: {e}"));
        eprintln!("PLX-113: Connectome written to {out}");
    }
}
