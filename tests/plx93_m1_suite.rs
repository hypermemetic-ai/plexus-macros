//! PLX-93 (M1·I) — the M1 reference suite's uncovered T-IDs.
//!
//! PLX-73's "Validation TDD" section defines M1-T1..T8 as pseudocode written
//! BEFORE any code existed, with an explicit comparison rule: every actual test
//! maps to a T-ID (or is justified as extra), and every T-ID is covered by at
//! least one actual test. This file closes the T-IDs that had no test at all.
//! The full matrix, both directions, is recorded as a comment on PLX-93.
//!
//! What is here:
//!
//! | T-ID | test |
//! |---|---|
//! | M1-T1 (amended) | `m1_t1_a_macro_produced_document_with_every_edge_kind_round_trips` |
//! | M1-T5 (amended) | `m1_t5_a_streaming_turn_projects_n_updates_a_terminal_and_done` |
//! | M1-T5 (amended) | `m1_t5_the_projection_is_the_documented_table_not_an_accident` |
//! | M1-T8 (amended) | `m1_t8_an_extensions_free_methods_wire_schema_is_byte_pinned` |
//! | M1-T8 (amended) | `m1_t8_the_pinned_schema_carries_exactly_the_0_6_key_set` |
//!
//! M1-T3 is in `src/plx93_seam_tests.rs` (it needs the expansion, not the
//! runtime). M1-T6 and M1-T7 are in `tests/plx93_m1_compile_tests.rs`.

use futures::StreamExt;
use futures::stream::Stream;
use plexus_core::plexus::{Activation, PlexusStreamItem};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Fixture. Deliberately minimal and extensions-free: `ping` declares no
// `ext(...)`, no `scope`, no `public`, no `http_method`, no parameters. It is
// the smallest method the grammar can express, which is what makes it a usable
// wire pin — anything that moves in its serialized schema moved for everyone.
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Pin93;

#[plexus_macros::activation(
    namespace = "pin93",
    version = "1.0.0",
    description = "The PLX-93 wire-pin fixture.",
    crate_path = "plexus_core"
)]
impl Pin93 {
    /// Say pong.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "pong".to_string(); }
    }

    /// Say pong three times.
    #[plexus_macros::method]
    async fn tick(&self) -> impl Stream<Item = u32> + Send + 'static {
        async_stream::stream! {
            yield 1u32;
            yield 2u32;
            yield 3u32;
        }
    }
}

// ===========================================================================
// M1-T5 — AMENDED. See PLX-73's suite for the dated amendment note.
//
// PLX-73 wrote:
//
//     M1-T5 streaming_kept:  fn -> impl Stream<Item=T> yields N data + done,
//                            byte-shape equal to 0.6 for same events
//
// The first clause holds. The second was invalidated INSIDE PLX-73 itself,
// three sections below the suite: `q-acp-as-communication-model` was answered
// ADOPT on 2026-07-25, making every call a TURN — zero or more updates plus
// exactly one terminal carrying a stop reason. A terminal is a wire event 0.6
// did not have, so byte-shape equality with 0.6 is not a property M1 can hold
// and still be M1. The suite's own pseudocode simply predates the decision.
//
// What replaces it is not weaker, it is *stated*: `plexus_core::runtime::bridge`
// documents the projection from turn events onto the legacy `PlexusStreamItem`
// as a three-row table, and these tests pin that table. "No silent drift"
// survives; "identical to 0.6" does not.
// ===========================================================================

/// M1-T5, first clause (unchanged): a streaming method still yields N data
/// items and a `Done`.
#[tokio::test]
async fn m1_t5_a_streaming_turn_projects_n_updates_a_terminal_and_done() {
    let items = dispatch("tick", serde_json::json!({})).await;

    // N updates …
    let updates: Vec<&PlexusStreamItem> = items
        .iter()
        .filter(|i| matches!(i, PlexusStreamItem::Data { content_type, .. } if content_type.ends_with(".update")))
        .collect();
    assert_eq!(updates.len(), 3, "three yields must produce three updates");

    // … then exactly one terminal …
    let terminals: Vec<&PlexusStreamItem> = items
        .iter()
        .filter(|i| matches!(i, PlexusStreamItem::Data { content_type, .. } if content_type.ends_with(".terminal")))
        .collect();
    assert_eq!(terminals.len(), 1, "a turn resolves exactly once");

    // … then `Done`, last.
    assert!(
        matches!(items.last(), Some(PlexusStreamItem::Done { .. })),
        "the stream must end with Done, got {:?}",
        items.last()
    );

    // Ordering: every update precedes the terminal.
    let terminal_at = items
        .iter()
        .position(|i| matches!(i, PlexusStreamItem::Data { content_type, .. } if content_type.ends_with(".terminal")))
        .unwrap();
    for (idx, item) in items.iter().enumerate() {
        if matches!(item, PlexusStreamItem::Data { content_type, .. } if content_type.ends_with(".update")) {
            assert!(idx < terminal_at, "an update appeared after the terminal");
        }
    }
}

/// M1-T5, second clause (amended): the wire shape is the DOCUMENTED
/// projection, pinned exactly, rather than "the same as 0.6".
///
/// Both halves of the drift are named here so neither can happen quietly:
///
/// - `content_type` is `<dotted_id>.update`, where 0.6 emitted
///   `<namespace>.<method>` for every data item (`wrap_stream`'s second
///   argument; see `plexus_core::plexus::streaming`).
/// - a `terminal` data item exists at all, carrying the turn's `StopReason`.
///   0.6 had no such event; a client inferred "why it ended" from whether the
///   last item happened to be an error.
#[tokio::test]
async fn m1_t5_the_projection_is_the_documented_table_not_an_accident() {
    let items = dispatch("ping", serde_json::json!({})).await;

    let types: Vec<&str> = items
        .iter()
        .filter_map(|i| match i {
            PlexusStreamItem::Data { content_type, .. } => Some(content_type.as_str()),
            _ => None,
        })
        .collect();

    assert_eq!(
        types,
        vec!["pin93.ping.update", "pin93.ping.terminal"],
        "the projection is `<dotted_id>.update` then `<dotted_id>.terminal`"
    );

    // The update carries the handler's own value, unwrapped.
    let update = items
        .iter()
        .find_map(|i| match i {
            PlexusStreamItem::Data { content_type, content, .. }
                if content_type == "pin93.ping.update" =>
            {
                Some(content.clone())
            }
            _ => None,
        })
        .expect("an update");
    assert_eq!(update, serde_json::json!("pong"));

    // The terminal carries a stop reason — the thing 0.6 could not say.
    let terminal = items
        .iter()
        .find_map(|i| match i {
            PlexusStreamItem::Data { content_type, content, .. }
                if content_type == "pin93.ping.terminal" =>
            {
                Some(content.clone())
            }
            _ => None,
        })
        .expect("a terminal");
    assert_eq!(
        terminal["stop"]["kind"],
        serde_json::json!("complete"),
        "the terminal must carry the turn's stop reason, got {terminal}"
    );
}

// ===========================================================================
// M1-T8 — AMENDED. See PLX-73's suite for the dated amendment note.
//
// PLX-73 wrote:
//
//     M1-T8 wire_pin:  extensions-free unary method's MethodSchema JSON
//                      byte-equals the 0.6 pin fixture   # no silent wire drift
//
// Two things moved under it, and one of them is the one PLX-93 was told to
// look for:
//
// 1. **There is no 0.6 pin fixture for a MethodSchema.** The only byte-equality
//    golden anywhere in the two repos is `plexus-core/tests/golden/solar.json`,
//    and it is (a) a whole `PluginSchema`, not a `MethodSchema`, (b) captured at
//    IR-3 tip rather than 0.6, and (c) exercised only as a *serializer
//    stability* round-trip — read, deserialize, re-serialize, compare to disk —
//    which cannot catch the macro emitting a different schema. Its own doc
//    comment concedes this. So M1-T8 could not be discharged as written: the
//    fixture it compares against does not exist.
//
// 2. **"unary" is not spellable on the `#[activation]` grammar yet.**
//    `#[activation]` cannot compile a `-> Result<T, E>` method at all — the
//    handler codegen drives every return value as a `Stream`. That is PLX-110
//    (M2·B0), in flight on branch `plx-110-unary-handler`, and PLX-93 does not
//    duplicate it. The pin below therefore uses the smallest method the grammar
//    CAN express: extensions-free, parameterless, one value.
//
// 3. CONNECTOME-HASH/1 (RFC 002 §4.6) replaced the hash construction and
//    changed every IR hash. It did NOT touch `MethodSchema.hash`, which is
//    still the legacy `DefaultHasher` signature hash — the two are different
//    objects and PLX-91's amended requirement 4 records why they stay that way.
//    So the wire pin below is over `MethodSchema` and is unaffected by §4.6;
//    the §4.6 construction is pinned separately and byte-exactly by
//    `connectome_hash_1_framing_golden` in `plexus-core/src/ir/hash.rs`.
// ===========================================================================

/// The pin. Regenerate deliberately with
/// `PLX93_PIN_OUT=tests/golden/plx93_extensions_free_method_schema.json cargo
/// test --test plx93_m1_suite m1_t8` and review the diff — that is the whole
/// point of the fixture.
const PIN: &str = include_str!("golden/plx93_extensions_free_method_schema.json");

/// M1-T8, first clause: byte equality. No silent wire drift.
#[test]
fn m1_t8_an_extensions_free_methods_wire_schema_is_byte_pinned() {
    let produced = ping_schema_json();

    if let Ok(path) = std::env::var("PLX93_PIN_OUT") {
        std::fs::write(&path, format!("{produced}\n")).expect("the pin is writable");
        eprintln!("PLX-93: wire pin written to {path}");
        return;
    }

    let expected = PIN.trim_end_matches('\n');
    if expected != produced {
        eprintln!("--- pinned (expected) ---\n{expected}\n");
        eprintln!("--- produced (actual)  ---\n{produced}\n");
        panic!(
            "the MethodSchema wire shape for an extensions-free method changed. \
             If that is intended, regenerate with PLX93_PIN_OUT=… and say why in \
             the commit."
        );
    }
}

/// M1-T8, second clause, AMENDED: the pin's provenance.
///
/// A golden captured today proves only that today equals today. What makes it a
/// *0.6* pin is that its key set is exactly the key set the 0.6-era capture
/// carries for the same method shape — `plexus-core/tests/golden/solar.json`,
/// taken from substrate's Solar activation at IR-3 tip, method `observe`
/// (parameterless, extensions-free):
///
/// ```text
/// bidirectional, description, hash, http_method, name, returns, role, streaming
/// ```
///
/// If M1 had added a field, dropped one, or changed a `skip_serializing_if`,
/// this is where it shows up.
#[test]
fn m1_t8_the_pinned_schema_carries_exactly_the_0_6_key_set() {
    // Verbatim from plexus-core/tests/golden/solar.json, method `observe`.
    // Solar 1.0.0, plexus-macros commit b6285a8.
    let zero_six: Vec<&str> = vec![
        "bidirectional",
        "description",
        "hash",
        "http_method",
        "name",
        "returns",
        "role",
        "streaming",
    ];

    let produced: serde_json::Value =
        serde_json::from_str(&ping_schema_json()).expect("the produced schema is JSON");
    let mut keys: Vec<&str> = produced
        .as_object()
        .expect("a MethodSchema is a JSON object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();

    assert_eq!(
        keys, zero_six,
        "the wire key set for an extensions-free parameterless method must be \
         exactly what 0.6 emitted; a difference here is wire drift, not a \
         refactor"
    );
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

/// `ping`'s `MethodSchema`, serialized the way the golden is stored.
fn ping_schema_json() -> String {
    let schema = Pin93.plugin_schema();
    let ping = schema
        .methods
        .iter()
        .find(|m| m.name == "ping")
        .expect("the fixture declares `ping`");
    serde_json::to_string_pretty(ping).expect("a MethodSchema serializes")
}

/// Dispatch a method through the owned entry point and collect the projection.
async fn dispatch(method: &str, params: serde_json::Value) -> Vec<PlexusStreamItem> {
    let activation = Arc::new(Pin93);
    let stream = Activation::call_arc(activation, method, params, None, None)
        .await
        .expect("dispatch succeeds");
    stream.collect().await
}

// ===========================================================================
// M1-T1 — the remaining half. See PLX-73's suite for the dated amendment note.
//
// PLX-73 wrote:
//
//     M1-T1 ir_roundtrip:  for each substrate activation A:
//                            ir = derive_ir(A)
//                            assert deserialize(serialize(ir)) == ir
//
// "for each substrate activation" is not reachable from plexus-macros or
// plexus-core: neither can depend on plexus-substrate (plexus-core cannot at
// all — it would be a cycle — and that is the same reason
// `plexus-core/tests/golden_schema.rs` says it can only *approximate* substrate
// byte-identity). The clause lands in M2, where substrate is on vNext and its
// own suite runs; M2-T4 `suite_green` is where it becomes checkable.
//
// What IS reachable, and was missing: a round-trip of a document produced by
// `#[activation]` — the shipped path — carrying ALL THREE edge kinds. The
// existing coverage splits the two halves and never joins them:
//
//   * `plx79_ir_parse_tests::the_document_round_trips_and_its_hashes_are_merkle`
//     round-trips all three edge kinds, but from `activation_ir!`, the
//     standalone parser macro, not from `#[activation]`.
//   * `plx91_switchover_tests::the_switched_macro_exports_a_connectome`
//     round-trips a `#[activation]` document, but it has exactly one Static
//     edge and no Dynamic or Indexed edge at all.
//
// So no test round-tripped an `#[activation]`-produced document with a Dynamic
// or Indexed edge in it. This one does, and it doubles as the richer document
// PLX-93's criterion 4 hands to connectome-hs.
// ===========================================================================

#[derive(Clone)]
struct T1Leaf;

#[plexus_macros::activation(
    namespace = "t1_leaf",
    version = "1.0.0",
    description = "A leaf activation.",
    crate_path = "plexus_core"
)]
impl T1Leaf {
    /// Say pong.
    #[plexus_macros::method]
    async fn ping(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "pong".to_string(); }
    }
}

#[derive(Clone)]
struct T1Root {
    embedded: T1Leaf,
}

#[plexus_macros::activation(
    namespace = "t1_root",
    version = "1.0.0",
    description = "A root carrying every edge kind.",
    crate_path = "plexus_core"
)]
impl T1Root {
    /// List the sessions.
    #[plexus_macros::method]
    async fn list(&self) -> impl Stream<Item = String> + Send + 'static {
        async_stream::stream! { yield "s1".to_string(); }
    }

    /// A Static edge — an embedded subtree.
    #[plexus_macros::child]
    fn embedded(&self) -> T1Leaf {
        self.embedded.clone()
    }

    /// An Indexed edge — instances enumerated by `list`.
    #[plexus_macros::child(list = "list")]
    fn session(&self) -> T1Leaf {
        T1Leaf
    }

    /// A Dynamic edge — the unmatched-name fallback.
    #[plexus_macros::child]
    async fn plugins(&self, _name: &str) -> Option<T1Leaf> {
        Some(T1Leaf)
    }
}

/// M1-T1: a document produced by `#[activation]`, carrying a Static, an
/// Indexed and a Dynamic edge, survives serialize → deserialize unchanged —
/// hashes included.
#[test]
fn m1_t1_a_macro_produced_document_with_every_edge_kind_round_trips() {
    use plexus_core::ir::{ActivationIr, ChildEdge};

    let ir = T1Root::activation_ir();

    // The fixture is not vacuous: all three edge kinds are really present.
    let mut static_edges = 0;
    let mut dynamic_edges = 0;
    let mut indexed_edges = 0;
    for edge in &ir.children {
        match edge {
            ChildEdge::Static { .. } => static_edges += 1,
            ChildEdge::Dynamic { .. } => dynamic_edges += 1,
            ChildEdge::Indexed { .. } => indexed_edges += 1,
        }
    }
    assert_eq!(
        (static_edges, dynamic_edges, indexed_edges),
        (1, 1, 1),
        "the fixture must exercise all three edge kinds, got {:?}",
        ir.children
    );

    // §3.4 — lossless.
    let json = serde_json::to_string_pretty(ir).expect("the document serializes");
    let back: ActivationIr = serde_json::from_str(&json).expect("the document decodes");
    assert_eq!(&back, ir, "the round trip must be lossless");

    // …and the hashes are part of "unchanged", not recomputed on the way back.
    assert!(ir.ir_hash.is_some(), "a root document carries an ir_hash");
    assert_eq!(back.ir_hash, ir.ir_hash);
    assert_eq!(back.hash, ir.hash);

    // The export hook for PLX-93 criterion 4: connectome-hs checks THIS
    // document, produced end to end by the switched macro.
    if let Ok(out) = std::env::var("PLX93_CONNECTOME_OUT") {
        std::fs::write(&out, &json).unwrap_or_else(|e| panic!("could not write {out}: {e}"));
        eprintln!("PLX-93: Connectome written to {out}");
    }
}
