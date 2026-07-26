//! PLX-90 (M1·B) acceptance tests: the parser agrees with **CONNECTOME RFC
//! 002**.
//!
//! PLX-79 was written against the pre-002 IR. RFC 002 then added the request
//! context (§6.9), replaced the hash construction with `CONNECTOME-HASH/1`
//! (§4.6), made `ir_version` optional and `hash_algorithm` a root fact (§3.3),
//! and ruled methods and child edges to be **sets** (§4.8). Every one of those
//! changes was *additive*, so the parser kept compiling and the drift was
//! silent. These tests are the thing that makes it loud.
//!
//! Run with: `cargo test --test plx90_rfc002_reconcile_tests`.

// The marker structs below exist only to give `activation_ir!` an impl block to
// read; nothing constructs them.
#![allow(dead_code)]

use plexus_core::ir::{ActivationIr, ChildEdge};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

// --- fixture payload types -------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Answer {
    text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct Update {
    chunk: String,
}

/// The transport-observable facts an activation's methods depend on
/// (RFC 002 §6.9.1) — headers, origin, peer, trace id. Note what it is *not*:
/// a parameter struct. Nothing here is assembled by the caller.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct HttpContext {
    /// `Origin` request header.
    origin: String,
    /// W3C trace identifier.
    trace_id: String,
}

/// A different set of transport facts, declared by one method that needs
/// something else. §6.9.2: it REPLACES, it does not merge.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
struct AdminContext {
    /// Presented bearer credential.
    authorization: String,
}

// ===========================================================================
// Criterion 1 — request context is emitted, with §6.9 semantics
// ===========================================================================

struct Ctxd;

fn request_context_fixture() -> ActivationIr {
    plexus_macros::activation_ir!(
        namespace = "ctxd",
        version = "1.0.0",
        backend_name = "substrate",
        request = HttpContext,
        /// An activation whose methods depend on transport facts.
        impl Ctxd {
            /// Inherits the activation's declaration (§6.9.3).
            #[method]
            async fn inherits(&self, id: String) -> Result<Answer, Answer> {
                todo!()
            }

            /// Declares its own, which REPLACES the activation's (§6.9.2).
            #[method(request = AdminContext)]
            async fn overrides(&self, id: String) -> Result<Answer, Answer> {
                todo!()
            }
        }
    )
    .expect("ActivationIr builds")
}

/// An activation that declares nothing, whose methods therefore need nothing
/// (§6.9.4) — absence is a statement, not a gap.
struct Plain;

impl Plain {
    fn __plexus_activation_ir() -> Result<ActivationIr, plexus_core::ir::SchemaRefError> {
        plexus_macros::activation_ir!(
            namespace = "plain",
            version = "1.0.0",
            impl Plain {
                #[method]
                async fn ping(&self) -> Result<Answer, Answer> {
                    todo!()
                }
            }
        )
    }
}

#[test]
fn request_context_is_emitted_at_both_levels_with_rfc002_semantics() {
    let ir = request_context_fixture();

    // §6.9.1 — the activation carries its declaration.
    let act = ir.request_context.as_ref().expect("activation declares one");
    assert_eq!(act.type_name(), "HttpContext");

    let inherits = ir.method("inherits").unwrap();
    let overrides = ir.method("overrides").unwrap();

    // §6.9.3 — the inheriting method stores NOTHING of its own; resolution is
    // the IR's job, and it resolves to the activation's.
    assert!(inherits.request_context.is_none());
    assert_eq!(
        ir.effective_request_context(inherits).unwrap().type_name(),
        "HttpContext"
    );

    // §6.9.2 — the overriding method REPLACES. No merge, no union.
    assert_eq!(
        overrides.request_context.as_ref().unwrap().type_name(),
        "AdminContext"
    );
    assert_eq!(
        ir.effective_request_context(overrides).unwrap().type_name(),
        "AdminContext",
        "the method's declaration replaces the activation's outright"
    );

    // §6.9.5 — a request context is NEVER a parameter. Both methods declare
    // exactly the one input their signature declares.
    for m in [inherits, overrides] {
        assert_eq!(
            m.params.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            ["id"],
            "`{}` must not have request-context fields spliced into params",
            m.dotted_id
        );
    }

    // §6.9.4 — absent at both levels means "needs nothing", positively.
    let plain = Plain::__plexus_activation_ir().unwrap();
    let ping = plain.method("ping").unwrap();
    assert!(plain.request_context.is_none());
    assert!(ping.request_context.is_none());
    assert!(plain.effective_request_context(ping).is_none());
}

#[test]
fn request_context_is_content_and_participates_in_the_hash() {
    // §6.9.6 — changing what a method demands of the transport is a change to
    // the method, and therefore to its ancestors.
    let with = request_context_fixture();

    let mut without = with.clone();
    without.methods[1].request_context = None;
    without.recompute_hashes();

    assert_ne!(
        with.methods[1].hash, without.methods[1].hash,
        "the declaration is in the method preimage (§4.6)"
    );
    assert_ne!(with.hash, without.hash, "and folds up the tree (§4.2)");
    assert_ne!(with.ir_hash, without.ir_hash);
}

// ===========================================================================
// Criterion 2 — methods and child edges are SETS (§4.8)
// ===========================================================================

struct Alpha;
struct Beta;

impl Alpha {
    fn __plexus_activation_ir() -> Result<ActivationIr, plexus_core::ir::SchemaRefError> {
        plexus_macros::activation_ir!(
            namespace = "alpha",
            version = "1.0.0",
            impl Alpha {
                #[method]
                async fn a(&self) -> Result<Answer, Answer> { todo!() }
            }
        )
    }
}

impl Beta {
    fn __plexus_activation_ir() -> Result<ActivationIr, plexus_core::ir::SchemaRefError> {
        plexus_macros::activation_ir!(
            namespace = "beta",
            version = "1.0.0",
            impl Beta {
                #[method]
                async fn b(&self) -> impl Stream<Item = Update> { todo!() }
            }
        )
    }
}

struct OrderOne;
struct OrderTwo;

/// Declaration order: `one`, `two`; children `alpha`, `beta`.
fn order_one() -> ActivationIr {
    plexus_macros::activation_ir!(
        namespace = "ordered",
        version = "1.0.0",
        backend_name = "substrate",
        impl OrderOne {
            /// First.
            #[method]
            async fn one(&self, x: String) -> Result<Answer, Answer> { todo!() }

            /// Second.
            #[method]
            async fn two(&self) -> impl Stream<Item = Update> { todo!() }

            #[child]
            fn alpha(&self) -> Alpha { todo!() }

            #[child]
            fn beta(&self) -> Beta { todo!() }
        }
    )
    .expect("ActivationIr builds")
}

/// The same activation, every declaration order reversed.
fn order_two() -> ActivationIr {
    plexus_macros::activation_ir!(
        namespace = "ordered",
        version = "1.0.0",
        backend_name = "substrate",
        impl OrderTwo {
            #[child]
            fn beta(&self) -> Beta { todo!() }

            #[child]
            fn alpha(&self) -> Alpha { todo!() }

            /// Second.
            #[method]
            async fn two(&self) -> impl Stream<Item = Update> { todo!() }

            /// First.
            #[method]
            async fn one(&self, x: String) -> Result<Answer, Answer> { todo!() }
        }
    )
    .expect("ActivationIr builds")
}

#[test]
fn declaration_order_of_methods_and_child_edges_does_not_change_any_hash() {
    let a = order_one();
    let b = order_two();

    // The two documents really are declared in opposite orders...
    assert_eq!(
        a.methods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["one", "two"]
    );
    assert_eq!(
        b.methods.iter().map(|m| m.name.as_str()).collect::<Vec<_>>(),
        ["two", "one"]
    );
    assert_eq!(
        a.children.iter().map(|c| c.namespace()).collect::<Vec<_>>(),
        ["alpha", "beta"]
    );
    assert_eq!(
        b.children.iter().map(|c| c.namespace()).collect::<Vec<_>>(),
        ["beta", "alpha"]
    );

    // ...and hash byte-identically. §4.8: methods and child edges are SETS,
    // keyed by §3.7.1's local name and §3.7.3's namespace, so declaration order
    // is exactly the "non-canonical ordering" §4.4 forbids the hash from
    // depending on.
    assert_eq!(a.hash, b.hash, "root activation hash is order-independent");
    assert_eq!(a.ir_hash, b.ir_hash, "and so is the document digest");

    // Per-node too, not just at the root.
    let mha: std::collections::BTreeMap<_, _> =
        a.methods.iter().map(|m| (&m.name, &m.hash)).collect();
    let mhb: std::collections::BTreeMap<_, _> =
        b.methods.iter().map(|m| (&m.name, &m.hash)).collect();
    assert_eq!(mha, mhb);
}

// ===========================================================================
// Criterion 3 — §3.3 root-fact placement
// ===========================================================================

#[test]
fn a_root_carries_the_root_facts_and_an_embedded_child_carries_none() {
    let ir = order_one();

    // §3.3.1 — the three MUST facts, emitted even at their defaults, and §4.7's
    // declared construction identifier among them.
    assert_eq!(ir.ir_version, Some(plexus_core::ir::IR_VERSION));
    assert_eq!(ir.hash_algorithm.as_deref(), Some("CONNECTOME-HASH/1"));
    assert!(ir.ir_hash.is_some());
    assert_eq!(ir.backend_name.as_deref(), Some("substrate"));

    // §3.3 — "Non-root activations MUST NOT carry any of them; their presence
    // is what distinguishes a root from an embedded node." The children are
    // built by the *same* `activation_ir!` macro, so each is a root in its own
    // right until it is embedded; embedding must strip it.
    let ChildEdge::Static(child) = ir.child("alpha").expect("static edge") else {
        panic!("expected a Static edge");
    };
    assert!(child.ir_version.is_none(), "§3.3: no ir_version on a child");
    assert!(
        child.hash_algorithm.is_none(),
        "§3.3: no hash_algorithm on a child"
    );
    assert!(child.ir_hash.is_none(), "§3.3: a child is not a document");
    assert!(child.backend_name.is_none());
    assert!(child.respond_method.is_none());

    // ...while standing alone, the very same activation IS a root and carries
    // them. That is the distinction §3.3 is drawing.
    let standalone = Alpha::__plexus_activation_ir().unwrap();
    assert_eq!(standalone.ir_version, Some(plexus_core::ir::IR_VERSION));
    assert_eq!(
        standalone.hash_algorithm.as_deref(),
        Some("CONNECTOME-HASH/1")
    );
    assert!(standalone.ir_hash.is_some());

    // §4.6 — and its *activation* hash is identical either way, because the
    // root facts are deliberately not in the activation preimage. This is what
    // makes an embedded subtree's hash comparable with the hash it reports when
    // fetched on its own.
    assert_eq!(
        standalone.hash, child.hash,
        "root facts must not enter the activation preimage (§4.6)"
    );
}

// ===========================================================================
// Criterion 4 — export for the independent conformance checker
// ===========================================================================

/// The document handed to `plexus-connectome`'s checker.
///
/// Deliberately exercises everything RFC 002 touched: root facts (§3.3), a
/// declared hash construction (§4.7), request contexts at both levels (§6.9),
/// all three edge kinds (§5.1), and a streaming method.
struct Exported;

fn exported_document() -> ActivationIr {
    plexus_macros::activation_ir!(
        namespace = "claudecode",
        version = "2.0.0",
        backend_name = "substrate",
        respond_method = "respond",
        request = HttpContext,
        /// Drive Claude Code sessions.
        impl Exported {
            /// List sessions.
            #[method]
            async fn list(&self) -> Result<Answer, Answer> { todo!() }

            /// Administer the backend.
            #[method(request = AdminContext, http_method = "DELETE")]
            async fn purge(&self, confirm: bool) -> Result<Answer, Answer> { todo!() }

            /// Stream a reply.
            #[method]
            async fn chat(&self, prompt: String) -> impl Stream<Item = Update> { todo!() }

            #[child]
            fn alpha(&self) -> Alpha { todo!() }

            #[child(list = "list")]
            fn beta(&self) -> Beta { todo!() }

            #[child]
            fn plugins(&self, name: &str) -> Option<Alpha> { todo!() }
        }
    )
    .expect("ActivationIr builds")
}

/// Where the exported document lands, so the Haskell checker can be pointed at
/// a real file. Overridable with `PLX90_EXPORT`.
fn export_path() -> std::path::PathBuf {
    match std::env::var_os("PLX90_EXPORT") {
        Some(p) => std::path::PathBuf::from(p),
        None => std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join("plx90")
            .join("parser_connectome.json"),
    }
}

#[test]
fn export_a_parser_produced_connectome_for_the_independent_checker() {
    let ir = exported_document();
    let json = serde_json::to_string_pretty(&ir).expect("serializes");

    // §3.4 — and it round-trips, which is the property the checker's decode
    // step relies on.
    let back: ActivationIr = serde_json::from_str(&json).expect("round-trips");
    assert_eq!(back, ir);

    // §3.6 item 2 — the root facts are fields on the root activation object
    // itself, not a wrapper. This is exactly the shape the checker decodes.
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let o = v.as_object().expect("an activation is a JSON object");
    assert_eq!(o["ir_version"], serde_json::json!(1));
    assert_eq!(o["hash_algorithm"], serde_json::json!("CONNECTOME-HASH/1"));
    assert!(o.contains_key("ir_hash"));
    assert!(o.contains_key("request_context"));

    let path = export_path();
    std::fs::create_dir_all(path.parent().unwrap()).expect("export dir");
    std::fs::write(&path, format!("{json}\n")).expect("export");
    eprintln!("PLX-90: exported parser-produced Connectome to {}", path.display());
}
